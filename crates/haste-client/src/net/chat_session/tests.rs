use super::*;
use bytes::Bytes;
use haste_protocol::haste_core::text::Component;
use haste_protocol::wire::IdOr;

fn player_chat(sig: u8) -> PlayerChat {
    PlayerChat {
        global_index: 0,
        sender: Uuid::from_u128(1),
        index: 0,
        signature: Some(Bytes::from(vec![sig; 256])),
        message: "hi".into(),
        timestamp: 0,
        salt: 0,
        previous: vec![],
        unsigned_content: None,
        filter_type: 0,
        filter_bits: None,
        chat_type: IdOr::Id(0),
        sender_name: Component::text("a"),
        target_name: None,
    }
}

fn chat() -> ServerboundPlay {
    ServerboundPlay::Chat(sb::Chat {
        message: "hello".into(),
        timestamp: 1,
        salt: 2,
        signature: None,
        message_count: 0,
        acknowledged: [0; 3],
        checksum: 0,
    })
}

#[test]
fn unsigned_chat_still_acknowledges_what_it_saw() {
    let mut c = ClientChat::new(Uuid::from_u128(9), None);
    let mut out = Vec::new();
    c.on_login(true, &mut out);
    assert!(out.is_empty(), "no keys: no session announced");
    c.on_player_chat(&player_chat(1), &mut out);
    let mut p = chat();
    c.pack(&mut p);
    let ServerboundPlay::Chat(p) = p else {
        panic!()
    };
    assert_eq!(p.signature, None);
    assert_eq!(p.message_count, 1);
    assert_eq!(p.acknowledged, [0, 0, 0x08]);
}

#[test]
fn chat_ack_after_sixty_four() {
    let mut c = ClientChat::new(Uuid::from_u128(9), None);
    let mut out = Vec::new();
    for n in 0..65 {
        c.on_player_chat(&player_chat(n), &mut out);
    }
    assert_eq!(out.len(), 1);
    assert!(matches!(
        out[0],
        ServerboundPlay::ChatAck(sb::ChatAck { message_count: 65 })
    ));
}
