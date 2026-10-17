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
        MessageSignature { bytes: [7; 256] },
        MessageSignature { bytes: [9; 256] },
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
    let signer =
        ChatSigner::from_private_key_der(Uuid::from_u128(1), Uuid::from_u128(2), doc.as_bytes())
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

fn sig(n: u8) -> MessageSignature {
    MessageSignature { bytes: [n; 256] }
}

/// A model of what the server checks (`LastSeenMessagesValidator.applyUpdate`): a list that
/// starts with 20 empty slots and grows by one pending entry per message sent to the client;
/// an update first drops `offset` entries from the front, then the first 20 must be
/// acknowledged exactly where the client still holds them.
struct ServerModel {
    tracked: Vec<Option<(MessageSignature, bool)>>,
}

impl ServerModel {
    fn new() -> Self {
        Self {
            tracked: vec![None; 20],
        }
    }
    fn send(&mut self, s: MessageSignature) {
        self.tracked.push(Some((s, true)));
    }
    fn apply(&mut self, u: &LastSeenUpdate) -> Result<Vec<MessageSignature>, String> {
        let offset = u.offset as usize;
        if offset > self.tracked.len() - 20 {
            return Err(format!("offset {offset} too far"));
        }
        self.tracked.drain(..offset);
        let bits = u32::from(u.acknowledged[0])
            | u32::from(u.acknowledged[1]) << 8
            | u32::from(u.acknowledged[2]) << 16;
        let mut seen = Vec::new();
        for i in 0..20 {
            let slot = &mut self.tracked[i];
            if bits & (1 << i) != 0 {
                let (s, _) = slot.ok_or(format!("acknowledged unknown slot {i}"))?;
                *slot = Some((s, false));
                seen.push(s);
            } else {
                if slot.is_some_and(|(_, pending)| !pending) {
                    return Err(format!("dropped acknowledged slot {i}"));
                }
                *slot = None;
            }
        }
        if u.checksum != 0 && u.checksum != last_seen_checksum(&seen) {
            return Err("checksum".into());
        }
        Ok(seen)
    }
}

#[test]
fn tracker_updates_pass_the_servers_checks() {
    let mut client = LastSeenMessagesTracker::default();
    let mut server = ServerModel::new();
    // Nothing seen yet: an empty update.
    let u = client.generate_and_apply_update();
    assert_eq!((u.offset, u.acknowledged, u.entries.len()), (0, [0; 3], 0));
    assert_eq!(
        u.checksum, 1,
        "empty list checksums to 1 (0 means unchecked)"
    );
    assert_eq!(server.apply(&u).unwrap(), vec![]);

    // Two messages arrive, we chat: both acknowledged, oldest first.
    for n in [1, 2] {
        server.send(sig(n));
        assert!(client.add_pending(sig(n), true));
    }
    let u = client.generate_and_apply_update();
    assert_eq!(u.offset, 2);
    assert_eq!(u.entries, vec![sig(1), sig(2)]);
    assert_eq!(
        u.acknowledged,
        [0, 0, 0b1100],
        "slots 18 and 19: the newest"
    );
    assert_eq!(server.apply(&u).unwrap(), vec![sig(1), sig(2)]);

    // Our own message comes back, then a deleted one and a hidden one.
    server.send(sig(3));
    client.add_pending(sig(3), true);
    server.send(sig(4));
    client.add_pending(sig(4), true);
    client.ignore_pending(&sig(4));
    server.send(sig(5));
    client.add_pending(sig(5), false);
    assert!(!client.add_pending(sig(5), false), "a repeat is ignored");
    assert_eq!(client.offset(), 3);
    let u = client.generate_and_apply_update();
    assert_eq!(u.entries, vec![sig(1), sig(2), sig(3)]);
    assert_eq!(server.apply(&u).unwrap(), u.entries);

    // More than the window: the oldest fall out on both sides.
    for n in 10..40 {
        server.send(sig(n));
        client.add_pending(sig(n), true);
    }
    let u = client.generate_and_apply_update();
    assert_eq!(u.offset, 30);
    assert_eq!(u.entries.len(), 20);
    assert_eq!(u.entries[0], sig(20));
    assert_eq!(u.acknowledged, [0xFF, 0xFF, 0x0F]);
    assert_eq!(server.apply(&u).unwrap(), u.entries);
}

#[test]
fn chat_ack_offset_is_taken_once() {
    let mut client = LastSeenMessagesTracker::default();
    let mut server = ServerModel::new();
    for n in 0..70 {
        server.send(sig(n));
        client.add_pending(sig(n), true);
    }
    // vanilla sends `chat_ack` with the offset once it passes 64 (markMessageAsProcessed).
    assert_eq!(client.offset(), 70);
    assert_eq!(client.take_offset(), 70);
    assert_eq!(client.offset(), 0);
    server.tracked.drain(..70);
    let u = client.generate_and_apply_update();
    assert_eq!(u.offset, 0);
    assert_eq!(server.apply(&u).unwrap(), u.entries);
}

#[test]
fn last_seen_checksum_matches_java() {
    // Expected values computed independently with Java's Arrays.hashCode semantics (signed
    // bytes, wrapping int) and LastSeenMessages.computeChecksum.
    assert_eq!(last_seen_checksum(&[]), 1);
    assert_eq!(last_seen_checksum(&[sig(255)]), 32);
    assert_eq!(last_seen_checksum(&[sig(255), sig(1)]), -31);
    assert_eq!(last_seen_checksum(&[sig(255), sig(1), sig(3)]), 64);
}

fn key_pair() -> (ProfileKeyPair, RsaPublicKey) {
    use rsa::pkcs8::EncodePublicKey;
    let (private, public) = test_keys();
    let keys = ProfileKeyPair {
        private_key_der: private.to_pkcs8_der().unwrap().as_bytes().to_vec(),
        public_key_der: public.to_public_key_der().unwrap().as_bytes().to_vec(),
        key_signature: vec![9; 512],
        expires_at_millis: 1_800_000_000_123,
        refreshed_after_millis: 1_799_000_000_456,
    };
    (keys, public)
}

#[test]
fn local_session_update_and_encoder() {
    let (keys, public) = key_pair();
    let session = LocalChatSession::with_session_id(Uuid::from_u128(77), keys.clone()).unwrap();
    let update = session.update_packet();
    assert_eq!(update.session.session_id, Uuid::from_u128(77));
    assert_eq!(
        update.session.expires_at, 1_800_000_000_123,
        "milliseconds, as signed"
    );
    assert_eq!(&update.session.public_key[..], &keys.public_key_der[..]);
    assert_eq!(&update.session.key_signature[..], &keys.key_signature[..]);
    round_trip(&update);

    // The encoder signs as the profile, in this session, from index 0.
    let profile = Uuid::from_u128(5);
    let mut encoder = session.message_encoder(profile);
    let mut tracker = LastSeenMessagesTracker::default();
    tracker.add_pending(sig(1), true);
    let mut chat = Chat {
        message: "hi".into(),
        timestamp: 1_700_000_000_999,
        salt: 3,
        signature: None,
        message_count: 0,
        acknowledged: [0; 3],
        checksum: 0,
    };
    encoder.pack_chat(&mut chat, &mut tracker);
    assert_eq!(chat.message_count, 1);
    assert_eq!(
        chat.acknowledged,
        [0, 0, 0x08],
        "slot 19: the newest of the window"
    );
    assert_eq!(chat.checksum, last_seen_checksum(&[sig(1)]));
    let reference = session.message_encoder(profile);
    let payload = reference.payload("hi", 1_700_000_000_999, 3, &[sig(1)]);
    public
        .verify(
            Pkcs1v15Sign::new::<Sha256>(),
            &Sha256::digest(&payload),
            &chat.signature.unwrap(),
        )
        .unwrap();
    assert_eq!(encoder.index(), 1);
}

#[test]
fn local_session_rejects_bad_keys() {
    let (keys, _) = key_pair();
    let mut wrong = keys.clone();
    wrong.public_key_der = key_pair().0.public_key_der;
    assert!(matches!(
        LocalChatSession::create(wrong),
        Err(SessionError::Mismatch)
    ));
    let mut garbage = keys;
    garbage.private_key_der = vec![1, 2, 3];
    assert!(matches!(
        LocalChatSession::create(garbage),
        Err(SessionError::PrivateKey(_))
    ));
}
