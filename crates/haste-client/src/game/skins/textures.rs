//! The `textures` profile property (authlib's `MinecraftServicesSessionService.unpackTextures`):
//! a base64 JSON payload naming the SKIN, CAPE and ELYTRA texture URLs, the skin's `model`
//! metadata (`slim`), and a Yggdrasil signature next to the value.

use base64::Engine as _;

/// One `MinecraftProfileTexture`: the URL, its hash and the metadata's `model`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTexture {
    pub url: String,
    /// `MinecraftProfileTexture.getHash`: the URL path's file name without extension.
    pub hash: String,
    /// `getMetadata("model")` (`slim` for slim skins; absent for wide ones).
    pub model: Option<String>,
}

/// `MinecraftProfileTextures`: the three texture slots and whether the property was signed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileTextures {
    pub skin: Option<ProfileTexture>,
    pub cape: Option<ProfileTexture>,
    pub elytra: Option<ProfileTexture>,
    /// The property carried a signature. Haste does not verify it against the services keys
    /// (authlib's `SignatureState.SIGNED` needs them); an unsigned property is `INSECURE`.
    pub signed: bool,
}

impl ProfileTextures {
    pub fn is_empty(&self) -> bool {
        self.skin.is_none() && self.cape.is_none() && self.elytra.is_none()
    }
}

/// The texture URL prefixes authlib accepts (`MinecraftServicesDiscoveryService
/// .isAllowedTextureDomain`: the discovery document's `getTexture` URIs without
/// `{textureId}`, which are Mojang's texture server).
const ALLOWED_TEXTURE_URLS: [&str; 2] = [
    "http://textures.minecraft.net/texture/",
    "https://textures.minecraft.net/texture/",
];

/// `unpackTextures`: decodes the property. A payload that does not decode, has no textures or
/// names a URL off Mojang's texture server gives the empty set (authlib logs and returns
/// `MinecraftProfileTextures.EMPTY`), so the profile keeps its default skin.
pub fn unpack_textures(value: &str, signature: Option<&str>) -> ProfileTextures {
    match parse(value) {
        Some(mut t) => {
            t.signed = signature.is_some_and(|s| !s.is_empty());
            t
        }
        None => ProfileTextures::default(),
    }
}

fn parse(value: &str) -> Option<ProfileTextures> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|e| tracing::warn!("could not decode textures payload: {e}"))
        .ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| tracing::warn!("could not decode textures payload: {e}"))
        .ok()?;
    let textures = json.get("textures")?.as_object()?;
    let mut out = ProfileTextures::default();
    for (kind, t) in textures {
        let Some(url) = t.get("url").and_then(|u| u.as_str()) else {
            // authlib rejects a texture without a URL like a disallowed one
            tracing::warn!("textures payload url is invalid: {t}");
            return None;
        };
        if !ALLOWED_TEXTURE_URLS.iter().any(|p| url.starts_with(p)) {
            tracing::warn!("textures payload url is invalid: {url}");
            return None;
        }
        let texture = ProfileTexture {
            url: url.to_owned(),
            hash: texture_hash(url)?,
            model: t
                .get("metadata")
                .and_then(|m| m.get("model"))
                .and_then(|m| m.as_str())
                .map(str::to_owned),
        };
        match kind.as_str() {
            "SKIN" => out.skin = Some(texture),
            "CAPE" => out.cape = Some(texture),
            "ELYTRA" => out.elytra = Some(texture),
            _ => {}
        }
    }
    Some(out)
}

/// `MinecraftProfileTexture.getHash`: `FilenameUtils.getBaseName(new URL(url).getPath())`.
pub fn texture_hash(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let name = parsed.path().rsplit('/').next().unwrap_or("");
    let base = match name.rfind('.') {
        Some(i) => &name[..i],
        None => name,
    };
    Some(base.to_owned())
}
