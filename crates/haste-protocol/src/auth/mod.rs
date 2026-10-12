//! Online-mode authentication: the encryption handshake and the Mojang
//! session-server join (minecraft.wiki "Java Edition protocol/Encryption").
//!
//! Microsoft/Xbox tokens are not obtained here: `haste-setup` (setup) produces
//! an [`MsaTokens`] (Minecraft access token + profile) and the connection
//! uses it for `session/minecraft/join`.

use num_bigint::BigInt;
use rsa::pkcs8::DecodePublicKey;
use rsa::rand_core::{OsRng, RngCore};
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use sha1::{Digest, Sha1};
use uuid::Uuid;

/// The session server endpoint a client posts to before answering an
/// encryption request in online mode.
pub const SESSION_JOIN_URL: &str = "https://sessionserver.mojang.com/session/minecraft/join";

/// Minecraft Java credentials from the Microsoft login flow.
#[derive(Clone, Debug)]
pub struct MsaTokens {
    /// Minecraft services access token (the "accessToken" of the join request).
    pub access_token: String,
    /// The Minecraft profile UUID.
    pub profile_uuid: Uuid,
    /// The Minecraft profile name.
    pub username: String,
}

/// Authentication failure.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The server's key could not be parsed or used.
    #[error("RSA: {0}")]
    Rsa(String),
    /// HTTP failure talking to the session server.
    #[error("session server request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The session server refused the join (status and body).
    #[error("session server refused the join: HTTP {0}: {1}")]
    Refused(u16, String),
}

/// Minecraft's SHA-1 "hex digest": the digest read as a signed two's
/// complement big integer, printed in base 16 with a minus sign if negative.
pub fn minecraft_hex_digest(digest: &[u8]) -> String {
    BigInt::from_signed_bytes_be(digest).to_str_radix(16)
}

/// The `serverId` hash for the session join: SHA-1 over the server id string,
/// the shared secret and the server's DER public key.
pub fn server_hash(server_id: &str, shared_secret: &[u8], public_key_der: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(server_id.as_bytes());
    h.update(shared_secret);
    h.update(public_key_der);
    minecraft_hex_digest(&h.finalize())
}

/// A fresh random 16-byte AES shared secret.
pub fn new_shared_secret() -> [u8; 16] {
    let mut s = [0u8; 16];
    OsRng.fill_bytes(&mut s);
    s
}

/// Encrypts `data` with the server's DER (`SubjectPublicKeyInfo`) RSA key,
/// PKCS#1 v1.5 padded, as the encryption response requires.
pub fn rsa_encrypt(public_key_der: &[u8], data: &[u8]) -> Result<Vec<u8>, AuthError> {
    let key = RsaPublicKey::from_public_key_der(public_key_der)
        .map_err(|e| AuthError::Rsa(e.to_string()))?;
    key.encrypt(&mut OsRng, Pkcs1v15Encrypt, data)
        .map_err(|e| AuthError::Rsa(e.to_string()))
}

/// Tells the session server we are joining the server identified by
/// `server_hash` (expects HTTP 204).
pub async fn join_session(tokens: &MsaTokens, server_hash: &str) -> Result<(), AuthError> {
    join_session_at(SESSION_JOIN_URL, tokens, server_hash).await
}

/// [`join_session`] against an explicit URL (tests).
pub async fn join_session_at(
    url: &str,
    tokens: &MsaTokens,
    server_hash: &str,
) -> Result<(), AuthError> {
    let body = serde_json::json!({
        "accessToken": tokens.access_token,
        "selectedProfile": tokens.profile_uuid.simple().to_string(),
        "serverId": server_hash,
    });
    let resp = reqwest::Client::new().post(url).json(&body).send().await?;
    let status = resp.status();
    if status.is_success() {
        Ok(())
    } else {
        let text = resp.text().await.unwrap_or_default();
        Err(AuthError::Refused(status.as_u16(), text))
    }
}

#[cfg(test)]
mod tests {
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
}
