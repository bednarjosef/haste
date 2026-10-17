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
