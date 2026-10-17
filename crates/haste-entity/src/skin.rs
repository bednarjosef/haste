//! What a player wears: vanilla's `PlayerSkin` (body texture, cape texture, wide or slim arms)
//! and `DefaultPlayerSkin` (the skin a profile gets without a downloaded one).
//!
//! The textures themselves are loaded elsewhere: default skins come from the player's jar
//! (`textures/entity/player/{slim,wide}/<name>.png`), downloaded skins and capes are
//! registered under `minecraft:skins/<sha1>` / `minecraft:capes/<sha1>` by the client's skin
//! manager (vanilla `SkinManager`). This module only names them and builds the cape layer's
//! model once per cape texture, so drawing a player allocates no model per frame.
use std::sync::{Arc, OnceLock};

use haste_core::model::CubeModel;
use haste_core::Identifier;
use uuid::Uuid;

/// `PlayerModelType`: the arm width a skin is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlayerModelType {
    /// 3-pixel arms (`PlayerModelType.SLIM`, services name `slim`).
    Slim,
    /// 4-pixel arms (`PlayerModelType.WIDE`, services name `default`).
    Wide,
}

impl PlayerModelType {
    /// `PlayerModelType.byLegacyServicesName`: the texture metadata's `model` value; anything
    /// but `slim` (including no metadata) is wide.
    pub fn by_legacy_services_name(name: Option<&str>) -> PlayerModelType {
        match name {
            Some("slim") => PlayerModelType::Slim,
            _ => PlayerModelType::Wide,
        }
    }

    pub fn is_slim(self) -> bool {
        self == PlayerModelType::Slim
    }
}

/// `PlayerSkin`: the body texture, the cape (as the cape layer's model, whose texture is the
/// cape texture), the arm model and whether the textures came signed (`secure`).
#[derive(Clone, Debug)]
pub struct PlayerSkin {
    pub body: Identifier,
    /// `PlayerSkin.cape` as `CapeLayer`'s model (`PlayerCapeModel`) textured with the cape;
    /// `None` without a cape (or when the recorder's model dump is missing).
    pub cape: Option<Arc<CubeModel>>,
    pub model: PlayerModelType,
    pub secure: bool,
}

impl PlayerSkin {
    /// A skin from its textures; the cape layer's model is built here, once.
    pub fn new(
        body: Identifier,
        cape: Option<&Identifier>,
        model: PlayerModelType,
        secure: bool,
    ) -> PlayerSkin {
        PlayerSkin {
            body,
            cape: cape.and_then(cape_model),
            model,
            secure,
        }
    }

    /// The cape texture, if the skin has a cape.
    pub fn cape_texture(&self) -> Option<&Identifier> {
        self.cape.as_ref().map(|m| &m.texture)
    }
}

/// The default skin names, in `DefaultPlayerSkin.DEFAULT_SKINS` order (all slim first, then
/// all wide, each list alphabetical).
const DEFAULT_NAMES: [&str; 9] = [
    "alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri",
];

/// The 18 default skins, built once (`DefaultPlayerSkin.DEFAULT_SKINS`).
fn default_skins() -> &'static [Arc<PlayerSkin>] {
    static SKINS: OnceLock<Vec<Arc<PlayerSkin>>> = OnceLock::new();
    SKINS.get_or_init(|| {
        [PlayerModelType::Slim, PlayerModelType::Wide]
            .into_iter()
            .flat_map(|model| {
                let kind = if model.is_slim() { "slim" } else { "wide" };
                DEFAULT_NAMES.iter().map(move |name| {
                    Arc::new(PlayerSkin {
                        body: Identifier::minecraft(&format!("entity/player/{kind}/{name}")),
                        cape: None,
                        model,
                        secure: true,
                    })
                })
            })
            .collect()
    })
}

/// Java's `UUID.hashCode`: the xor of the two halves, folded to 32 bits.
pub fn java_uuid_hash(uuid: Uuid) -> i32 {
    let v = uuid.as_u128();
    let hilo = ((v >> 64) as u64) ^ (v as u64);
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// `DefaultPlayerSkin.get(UUID)`: the profile id's hash, floor-modulo the 18 default skins.
pub fn default_skin(uuid: Uuid) -> Arc<PlayerSkin> {
    let skins = default_skins();
    skins[java_uuid_hash(uuid).rem_euclid(skins.len() as i32) as usize].clone()
}

/// `DefaultPlayerSkin.getDefaultSkin`: wide Steve.
pub fn steve() -> Arc<PlayerSkin> {
    default_skins()[9 + 6].clone()
}

/// `CapeLayer`'s model (`PlayerCapeModel.createCapeLayer`, the recorder's `player#cape` layer)
/// textured with `texture`. The layer's cube samples its texture at half height
/// (`addBox(..., texScaleU 1, texScaleV 0.5)` over a 64x64 layer: capes are 64x32 images); the
/// dump records the layer size but not that scale, so the height is set to 32 here.
pub fn cape_model(texture: &Identifier) -> Option<Arc<CubeModel>> {
    let Some(layer) =
        crate::model_dump::library().and_then(|l| l.get("minecraft:player#cape", 1.0))
    else {
        static WARNED: OnceLock<()> = OnceLock::new();
        WARNED.get_or_init(|| tracing::warn!("no minecraft:player#cape layer: capes not drawn"));
        return None;
    };
    let mut model = (*layer).clone();
    model.texture = texture.clone();
    model.tex_height = model.tex_width / 2;
    Some(Arc::new(model))
}
