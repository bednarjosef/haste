//! Skin manager tests: the textures property, the cache naming and the legacy skin conversion.
use std::path::Path;

use base64::Engine as _;
use image::{Rgba, RgbaImage};

use super::download::{cache_location, process_legacy_skin, TextureType};
use super::textures::{texture_hash, unpack_textures};

fn packed(json: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(json)
}

#[test]
fn textures_property_names_skin_model_and_cape() {
    let value = packed(
        r#"{"timestamp":1,"profileId":"069a79f444e94726a5befca90e38aaf5","profileName":"Notch",
        "textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/1a2b3c",
        "metadata":{"model":"slim"}},"CAPE":{"url":"https://textures.minecraft.net/texture/4d5e6f"}}}"#,
    );
    let t = unpack_textures(&value, Some("c2lnbmF0dXJl"));
    let skin = t.skin.expect("skin");
    assert_eq!(skin.url, "http://textures.minecraft.net/texture/1a2b3c");
    assert_eq!(skin.hash, "1a2b3c");
    assert_eq!(skin.model.as_deref(), Some("slim"));
    let cape = t.cape.expect("cape");
    assert_eq!(cape.hash, "4d5e6f");
    assert_eq!(cape.model, None);
    assert!(t.elytra.is_none());
    assert!(t.signed);
    // the same value without a signature is insecure
    assert!(!unpack_textures(&value, None).signed);
}

#[test]
fn textures_property_rejects_bad_payloads() {
    // a texture off Mojang's texture server empties the whole set (authlib's EMPTY)
    let evil = packed(
        r#"{"textures":{"SKIN":{"url":"http://textures.minecraft.net/texture/aa"},
        "CAPE":{"url":"http://evil.example/texture/1a2b3c"}}}"#,
    );
    assert!(unpack_textures(&evil, Some("x")).is_empty());
    // not base64, not JSON, no textures
    assert!(unpack_textures("%%%", None).is_empty());
    assert!(unpack_textures(&packed("not json"), None).is_empty());
    assert!(unpack_textures(&packed(r#"{"profileName":"x"}"#), None).is_empty());
    assert!(unpack_textures(&packed(r#"{"textures":{}}"#), None).is_empty());
}

#[test]
fn texture_hash_is_the_file_base_name() {
    assert_eq!(
        texture_hash("http://textures.minecraft.net/texture/abc123").as_deref(),
        Some("abc123")
    );
    assert_eq!(
        texture_hash("https://textures.minecraft.net/texture/skin.png?x=1").as_deref(),
        Some("skin")
    );
}

/// `TextureCache.registerTexture`: SHA-1 over the hash's UTF-16LE code units (expected value
/// computed independently: sha1 of "abc" encoded UTF-16LE).
#[test]
fn cache_location_follows_texture_cache() {
    let (id, file) = cache_location(Path::new("/c"), TextureType::Skin, "abc");
    let sha = "9f04f41a848514162050e3d68c1a7abb441dc2b5";
    assert_eq!(id.to_string(), format!("minecraft:skins/{sha}"));
    assert_eq!(file, Path::new("/c").join("9f").join(sha));
    let (cape, _) = cache_location(Path::new("/c"), TextureType::Cape, "abc");
    assert_eq!(cape.to_string(), format!("minecraft:capes/{sha}"));
}

#[test]
fn legacy_skins_grow_and_mirror_the_limbs() {
    let mut legacy = RgbaImage::from_pixel(64, 32, Rgba([10, 20, 30, 0]));
    // right leg top face's first pixel and the right arm front's first pixel
    legacy.put_pixel(4, 16, Rgba([255, 0, 0, 200]));
    legacy.put_pixel(44, 20, Rgba([0, 255, 0, 255]));
    let out = process_legacy_skin(legacy, "test").unwrap();
    assert_eq!(out.dimensions(), (64, 64));
    // copyRect(4, 16, +16, +32, 4x4, mirrored in x): (4, 16) lands at (20 + 3, 48)
    assert_eq!(*out.get_pixel(23, 48), Rgba([255, 0, 0, 255]));
    // copyRect(44, 20, -8, +32, 4x12, mirrored): (44, 20) lands at (36 + 3, 52)
    assert_eq!(*out.get_pixel(39, 52), Rgba([0, 255, 0, 255]));
    // base layers are opaque, the transparent hat stays as it was
    assert_eq!(out.get_pixel(0, 0).0[3], 255);
    assert_eq!(out.get_pixel(40, 4).0[3], 0);
    // outside the copied slots the new half is cleared
    assert_eq!(*out.get_pixel(0, 40), Rgba([0, 0, 0, 0]));
}

#[test]
fn legacy_opaque_hats_are_cleared() {
    let legacy = RgbaImage::from_pixel(64, 32, Rgba([1, 2, 3, 255]));
    let out = process_legacy_skin(legacy, "test").unwrap();
    assert_eq!(*out.get_pixel(40, 4), Rgba([1, 2, 3, 0]));
    // a modern 64x64 skin keeps its opaque hat
    let modern = RgbaImage::from_pixel(64, 64, Rgba([1, 2, 3, 255]));
    let out = process_legacy_skin(modern, "test").unwrap();
    assert_eq!(out.get_pixel(40, 4).0[3], 255);
}

#[test]
fn wrongly_sized_skins_are_discarded() {
    assert!(process_legacy_skin(RgbaImage::new(128, 128), "test").is_err());
    assert!(process_legacy_skin(RgbaImage::new(64, 48), "test").is_err());
}

/// The network path end to end: Notch's public profile from the session server, its textures
/// property unpacked, the skin downloaded and converted. Needs the internet; run with
/// `cargo test -p haste-client -- --ignored skins`.
#[test]
#[ignore = "network: fetches a public profile and skin from Mojang"]
fn downloads_a_public_skin() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let profile: serde_json::Value = reqwest::get(
            "https://sessionserver.mojang.com/session/minecraft/profile/069a79f444e94726a5befca90e38aaf5?unsigned=false",
        )
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        let prop = profile["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "textures")
            .unwrap();
        let t = unpack_textures(
            prop["value"].as_str().unwrap(),
            prop["signature"].as_str(),
        );
        assert!(t.signed);
        let skin = t.skin.expect("Notch has a skin");
        // scratch inside the workspace's target directory
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/skin-download-test");
        let (_, file) = cache_location(&dir, TextureType::Skin, &skin.hash);
        let image = super::download::download(file.clone(), skin.url.clone(), true)
            .await
            .unwrap();
        assert_eq!(image.dimensions(), (64, 64));
        assert!(file.is_file(), "the download is cached");
        // a second load comes from the cache
        let again = super::download::download(file, skin.url, true).await.unwrap();
        assert_eq!(again, image);
        let _ = std::fs::remove_dir_all(dir);
    });
}
