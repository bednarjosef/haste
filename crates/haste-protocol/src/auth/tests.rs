use super::*;
use rsa::pkcs8::EncodePublicKey;
use rsa::RsaPrivateKey;

fn sha1(s: &str) -> Vec<u8> {
    Sha1::digest(s.as_bytes()).to_vec()
}

#[test]
fn hex_digest_wiki_examples() {
    assert_eq!(
        minecraft_hex_digest(&sha1("Notch")),
        "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
    );
    assert_eq!(
        minecraft_hex_digest(&sha1("jeb_")),
        "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
    );
    assert_eq!(
        minecraft_hex_digest(&sha1("simon")),
        "88e16a1019277b15d58faf0541e11910eb756f6"
    );
}

#[test]
fn rsa_round_trip_with_generated_key() {
    let private = RsaPrivateKey::new(&mut OsRng, 1024).unwrap();
    let der = private.to_public_key().to_public_key_der().unwrap();
    let secret = new_shared_secret();
    let enc = rsa_encrypt(der.as_bytes(), &secret).unwrap();
    assert_eq!(enc.len(), 128);
    let dec = private.decrypt(Pkcs1v15Encrypt, &enc).unwrap();
    assert_eq!(dec, secret);
    assert!(rsa_encrypt(&[1, 2, 3], &secret).is_err());
}

/// The inputs of the oracle values below: secret byte `i` is `i * 17 + 3`, key byte `i` is
/// `i * 7 + 1` (162 bytes, the length of a 1024-bit key's DER), plaintext byte `i` is
/// `255 - i * 3`.
fn oracle_inputs() -> ([u8; 16], Vec<u8>, Vec<u8>) {
    let secret = std::array::from_fn(|i| (i * 17 + 3) as u8);
    let key = (0..162).map(|i| (i * 7 + 1) as u8).collect();
    let plain = (0..40).map(|i| (255 - i * 3) as u8).collect();
    (secret, key, plain)
}

#[test]
fn server_hash_matches_java() {
    // Expected values printed by the JDK running the steps of vanilla
    // `ClientHandshakePacketListenerImpl.handleHello` (`Crypt.digestData` over
    // `getBytes("ISO_8859_1")`, secret, key; `new BigInteger(digest).toString(16)`). They cover
    // a digest with a leading zero nibble (39 digits), a negative digest and a server id with
    // a Latin-1 char and one outside Latin-1 (sent as `?`).
    let (secret, key, _) = oracle_inputs();
    assert_eq!(
        server_hash("", &secret, &key),
        "d208f9296a75047c60fdb2c02090f60ddc98a4d"
    );
    assert_eq!(
        server_hash("abc", &secret, &key),
        "-6ac2ccd199a19e48a7d6962ad80538d506fefb1b"
    );
    assert_eq!(
        server_hash("\u{fc}\u{20ac}", &secret, &key),
        "4e39a642d51fcdc0d36164e22e6498550cafb5c0"
    );
}

#[tokio::test]
async fn connection_cipher_matches_java_aes_cfb8() {
    // Vanilla `Crypt.getCipher`: AES/CFB8/NoPadding keyed and IV'd with the shared secret.
    // Expected bytes from the JDK's cipher over the same plaintext in two updates (17 + 23
    // bytes): the stream state carries across writes.
    let (secret, _, plain) = oracle_inputs();
    let expected =
        "0111ab9778c4768055ccc429cee275bac19a0eb04421f8c06ea570f8d1c2735ce26c167ee7f5f582";
    let mut wire = Vec::new();
    {
        let mut enc = crate::transport::TcpNetworkEncoder::new(&mut wire);
        enc.set_encryption(&secret).unwrap();
        enc.write_frame(&plain[..17]).await.unwrap();
        enc.write_frame(&plain[17..]).await.unwrap();
        enc.flush().await.unwrap();
    }
    let hex: String = wire.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(hex, expected);
}

#[test]
fn api_hosts_default_to_mojang_and_take_overrides() {
    // One test touches the env vars, so parallel tests never see a half-set state.
    std::env::remove_var(SESSION_HOST_ENV);
    std::env::remove_var(SERVICES_HOST_ENV);
    assert_eq!(session_host(), MOJANG_SESSION_HOST);
    assert_eq!(services_host(), MOJANG_SERVICES_HOST);
    assert!(!hosts_overridden());
    assert_eq!(
        format!("{}/session/minecraft/join", session_host()),
        SESSION_JOIN_URL
    );
    std::env::set_var(SESSION_HOST_ENV, "http://127.0.0.1:1/");
    assert_eq!(session_host(), "http://127.0.0.1:1");
    assert!(!hosts_overridden(), "both hosts must be overridden");
    std::env::set_var(SERVICES_HOST_ENV, "http://127.0.0.1:2");
    assert!(hosts_overridden());
    assert_eq!(services_host(), "http://127.0.0.1:2");
    std::env::remove_var(SESSION_HOST_ENV);
    std::env::remove_var(SERVICES_HOST_ENV);
}
