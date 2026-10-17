use super::*;
use crate::packets::test_util::round_trip;
use rsa::pkcs8::EncodePrivateKey;
use rsa::rand_core::OsRng;
use rsa::RsaPublicKey;

fn test_keys() -> (RsaPrivateKey, RsaPublicKey) {
    let private = RsaPrivateKey::new(&mut OsRng, 2048).unwrap();
    let public = RsaPublicKey::from(&private);
    (private, public)
}

fn sample_last_seen() -> Vec<MessageSignature> {
    vec![
        MessageSignature {
            timestamp: 1000,
            bytes: [7; 256],
        },
        MessageSignature {
            timestamp: 2000,
            bytes: [9; 256],
        },
    ]
}

#[test]
fn sign_and_verify_over_exact_payload() {
    let (private, public) = test_keys();
    let session = Uuid::from_u128(0x1234);
    let sender = Uuid::from_u128(0x5678);
    let mut signer = ChatSigner::new(session, sender, private);
    let last_seen = sample_last_seen();

    let payload = signer.payload("hello", 1_700_000_000_123, 42, &last_seen);
    let signature = signer.sign("hello", 1_700_000_000_123, 42, &last_seen);

    // The exact byte layout, built independently of the signer.
    let mut expected = Vec::new();
    // (vanilla PlayerChatMessage.updateSignature: fixed-width big-endian ints, no VarInts)
    expected.extend_from_slice(&1i32.to_be_bytes());
    expected.extend_from_slice(sender.as_bytes());
    expected.extend_from_slice(session.as_bytes());
    expected.extend_from_slice(&0i32.to_be_bytes());
    expected.extend_from_slice(&42i64.to_be_bytes());
    expected.extend_from_slice(&1_700_000_000i64.to_be_bytes());
    expected.extend_from_slice(&5i32.to_be_bytes());
    expected.extend_from_slice(b"hello");
    expected.extend_from_slice(&2i32.to_be_bytes());
    expected.extend_from_slice(&[7u8; 256]);
    expected.extend_from_slice(&[9u8; 256]);
    assert_eq!(payload, expected);

    public
        .verify(
            Pkcs1v15Sign::new::<Sha256>(),
            &Sha256::digest(&payload),
            &signature,
        )
        .unwrap();
    assert_eq!(signer.index(), 1);
}

#[test]
fn message_index_increments_per_message() {
    let (private, _) = test_keys();
    let mut signer = ChatSigner::new(Uuid::from_u128(1), Uuid::from_u128(2), private);
    let last_seen = sample_last_seen();
    let first = signer.sign("a", 1, 1, &last_seen);
    let second = signer.sign("b", 2, 2, &last_seen);
    assert_eq!(signer.index(), 2);
    assert_ne!(first, second);
    // version (4) + sender (16) + session (16) = 36 bytes, then the int index.
    assert_eq!(
        signer.payload("a", 1, 1, &last_seen)[36..40],
        2i32.to_be_bytes(),
        "payload carries the current index"
    );
}

#[test]
fn sign_argument_verifies() {
    let (private, public) = test_keys();
    let mut signer = ChatSigner::new(Uuid::from_u128(1), Uuid::from_u128(2), private);
    // An argument is signed like a chat message whose content is the argument text, and
    // advances the index like one.
    let payload = signer.payload("hello", 5_000, 7, &[]);
    let signature = signer.sign_argument("hello", 5_000, 7, &[]);
    public
        .verify(
            Pkcs1v15Sign::new::<Sha256>(),
            &Sha256::digest(&payload),
            &signature,
        )
        .unwrap();
    assert_eq!(signer.index(), 1);
}

#[test]
fn signer_from_private_key_der() {
    let (private, _) = test_keys();
    let doc = private.to_pkcs8_der().unwrap();
    let signer = ChatSigner::from_private_key_der(
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        doc.as_bytes(),
    )
    .unwrap();
    assert_eq!(signer.session_id(), Uuid::from_u128(1));
    assert_eq!(signer.index(), 0);
}

#[test]
fn fill_chat_sets_signature() {
    let (private, public) = test_keys();
    let mut signer = ChatSigner::new(Uuid::from_u128(1), Uuid::from_u128(2), private);
    let mut chat = Chat {
        message: "hi".into(),
        timestamp: 123,
        salt: 456,
        signature: None,
        message_count: 0,
        acknowledged: [0; 3],
        checksum: 0,
    };
    let payload = signer.payload("hi", 123, 456, &sample_last_seen());
    signer.fill_chat(&mut chat, &sample_last_seen());
    let signature = chat.signature.clone().expect("signature filled");
    assert_eq!(signature.len(), 256);
    public
        .verify(
            Pkcs1v15Sign::new::<Sha256>(),
            &Sha256::digest(&payload),
            &signature,
        )
        .unwrap();
}

#[test]
fn chat_session_update_round_trips() {
    let update = ChatSessionUpdate::new(
        Uuid::from_u128(0xABCD),
        1_700_000_000_000,
        Bytes::from_static(&[1, 2, 3]),
        Bytes::from_static(&[4, 5]),
    );
    round_trip(&update);
}

#[test]
fn last_seen_window_and_ack() {
    let mut seen = LastSeenMessages::default();
    assert_eq!(seen.message_count(), 0);
    assert_eq!(seen.acknowledged_bytes(), [0; 3]);
    assert_eq!(seen.checksum(), 1);

    seen.push(MessageSignature {
        timestamp: 1,
        bytes: [1; 256],
    });
    seen.push(MessageSignature {
        timestamp: 2,
        bytes: [2; 256],
    });
    assert_eq!(seen.message_count(), 2);
    assert_eq!(seen.acknowledged_bytes(), [0; 3]);

    seen.acknowledge(0);
    assert_eq!(seen.message_count(), 1);
    assert_eq!(seen.acknowledged_bytes(), [0x01, 0, 0]);

    seen.acknowledge(1);
    assert_eq!(seen.message_count(), 0);
    assert_eq!(seen.acknowledged_bytes(), [0x03, 0, 0]);

    assert_eq!(seen.take_ack(), 0);
    assert_eq!(seen.acknowledged_bytes(), [0; 3]);
}

#[test]
fn last_seen_evicts_oldest_beyond_twenty() {
    let mut seen = LastSeenMessages::default();
    for i in 0..25 {
        seen.push(MessageSignature {
            timestamp: i,
            bytes: [i as u8; 256],
        });
    }
    // The window holds 20; evicted entries were unacknowledged, so the
    // unacknowledged count stays at the window size.
    assert_eq!(seen.message_count(), 20);
    assert_eq!(seen.entries().count(), 20);
    assert_eq!(seen.entries().next().unwrap().timestamp, 5);

    // Acknowledging the oldest surviving entry keeps the count exact.
    seen.acknowledge(0);
    assert_eq!(seen.message_count(), 19);
    assert_eq!(seen.acknowledged_bytes(), [0x01, 0, 0]);
}

#[test]
fn checksum_matches_vanillas_last_seen_checksum() {
    // Expected values computed independently with Java's Arrays.hashCode semantics
    // (signed bytes, wrapping int) and LastSeenMessages.computeChecksum.
    let mut seen = LastSeenMessages::default();
    assert_eq!(seen.checksum(), 1, "empty list: 1 (0 is reserved)");
    seen.push(MessageSignature {
        timestamp: 1,
        bytes: [255; 256],
    });
    assert_eq!(seen.checksum(), 32);
    seen.push(MessageSignature {
        timestamp: 2,
        bytes: [1; 256],
    });
    assert_eq!(seen.checksum(), -31);
    seen.push(MessageSignature {
        timestamp: 3,
        bytes: [3; 256],
    });
    assert_eq!(seen.checksum(), 64);
}
