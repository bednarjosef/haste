//! Registry-driven mob variants (26.3): cat, wolf, frog, pig, cow, chicken.
//!
//! Vanilla resolves these mobs' look from a *synchronized* variant registry
//! (`RegistryDataLoader.SYNCHRONIZED_REGISTRIES`): the entity's synched data carries a
//! `Holder<XVariant>` (a plain VarInt registry id on the wire, `ByteBufCodecs.holderRegistry`),
//! and the registry entry's NBT names the textures (and, for pig/cow/chicken, the model type:
//! normal/cold/warm — the temperature layers exist in the model dump as
//! `minecraft:cold_pig#main` and friends).
//!
//! The tables here are filled once at join from the server's `registry_data` (parsed by
//! `haste-client`'s `net::mob_variants`, like the painting variants). Unknown or mod-added
//! variant ids (an id with no entry, or an entry without the fields vanilla's have) fall back
//! to the default-variant row of the model table.
use haste_core::Identifier;

mod cube_mob;
mod dye;
mod metadata_texture;
mod overlay;
mod registry_model;
mod shulker;
mod synced_registry;
#[cfg(test)]
mod tests;
mod tropical_fish;

pub use cube_mob::scale_for;
pub use dye::{dye_tint, DYE_COLORS};
pub use metadata_texture::{metadata_model_for, texture_for};
pub use overlay::overlays_for;
pub use registry_model::model_for;
pub use shulker::shulker_texture;
pub use synced_registry::{MobVariants, VARIANT_REGISTRIES};
pub use tropical_fish::tropical_fish_model;

/// Metadata index of the variant field per entity type, from each vanilla class's
/// `defineSynchedData` chain (`SynchedEntityData.defineId` numbers the super chain in order:
/// Entity 0-7, LivingEntity 8-14, Mob 15, AgeableMob 16-17, Animal none, then the class's own
/// fields in declaration order).
pub mod metadata_index {
    /// `Pig.DATA_VARIANT_ID`: boost time 18, variant 19 (Pig.java).
    pub const PIG: u8 = 19;
    /// `Cow.DATA_VARIANT_ID`: variant 18, sound variant 19 (Cow.java; AbstractCow adds none).
    pub const COW: u8 = 18;
    /// `Chicken.DATA_VARIANT_ID`: variant 18, sound variant 19 (Chicken.java).
    pub const CHICKEN: u8 = 18;
    /// `Frog.DATA_VARIANT_ID`: variant 18, tongue target 19 (Frog.java).
    pub const FROG: u8 = 18;
    /// `Cat.DATA_VARIANT_ID`: TamableAnimal 18-19 (flags, owner), variant 20, sound variant
    /// 21 (Cat.java).
    pub const CAT: u8 = 20;
    /// `Wolf.DATA_VARIANT_ID`: TamableAnimal 18-19, interested 20, collar 21, anger 22,
    /// variant 23 (Wolf.java).
    pub const WOLF: u8 = 23;
    /// `Axolotl.DATA_VARIANT`: variant 18, playing dead 19, from bucket 20 (Axolotl.java).
    pub const AXOLOTL: u8 = 18;
    /// `Rabbit.DATA_TYPE_ID`: the rabbit's variant 18 (Rabbit.java).
    pub const RABBIT: u8 = 18;
    /// `Horse.DATA_ID_TYPE_VARIANT`: AbstractHorse's flags 18, then the packed
    /// variant/markings int 19 (Horse.java: `variant = id & 0xFF`, `markings = id >> 8`).
    pub const HORSE_TYPE_VARIANT: u8 = 19;
    /// `Llama.DATA_VARIANT_ID`: AbstractHorse 18, AbstractChestedHorse's chest 19,
    /// strength 20, variant 21 (Llama.java).
    pub const LLAMA_VARIANT: u8 = 21;
    /// `Parrot.DATA_VARIANT_ID`: TamableAnimal 18-19, variant 20 (Parrot.java).
    pub const PARROT: u8 = 20;
    /// `Fox.DATA_TYPE_ID`: variant 18, flags 19, then the two trusted ids 20-21 (Fox.java's
    /// field declaration order — `defineId` numbers the fields, `defineSynchedData` only
    /// gives defaults).
    pub const FOX_TYPE: u8 = 18;
    /// `Fox.DATA_FLAGS_ID` (sitting 1, crouching 4, interested 8, pouncing 16,
    /// sleeping 32, faceplanted 64, defending 128).
    pub const FOX_FLAGS: u8 = 19;
    /// `Panda.MAIN_GENE_ID`: unhappy 18, sneeze 19, eat 20, main gene 21, hidden gene 22,
    /// flags 23, eat counter 24 (Panda.java).
    pub const PANDA_MAIN_GENE: u8 = 21;
    /// `Panda.HIDDEN_GENE_ID`.
    pub const PANDA_HIDDEN_GENE: u8 = 22;
    /// `AbstractCubeMob.ID_SIZE` (slime, magma cube): AgeableMob 16-17, size 18.
    pub const CUBE_SIZE: u8 = 18;
    /// `Bee.DATA_FLAGS_ID`: flags 18 (nectar 8), anger end time 19.
    pub const BEE_FLAGS: u8 = 18;
    /// `Bee.DATA_ANGER_END_TIME` (game-time tick the anger ends, -1 when calm).
    pub const BEE_ANGER_END: u8 = 19;
    /// `Strider.DATA_SUFFOCATING`: boost time 18, suffocating (cold) 19.
    pub const STRIDER_SUFFOCATING: u8 = 19;
    /// `MushroomCow.DATA_TYPE`: variant 18 (red 0, brown 1; AbstractCow adds nothing).
    pub const MOOSHROOM_TYPE: u8 = 18;
    /// `Sheep.DATA_WOOL_ID`: AgeableMob 16-17, then the packed wool byte 18 (colour =
    /// `byte & 15`, sheared = `byte & 16`).
    pub const SHEEP_WOOL: u8 = 18;
    /// `Horse.DATA_ID_TYPE_VARIANT`'s markings nibble: `id >> 8` (Markings.NONE 0, WHITE 1,
    /// WHITE_FIELD 2, WHITE_DOTS 3, BLACK_DOTS 4).
    pub const HORSE_MARKINGS_SHIFT: u32 = 8;
    /// `Cat.DATA_COLLAR_COLOR`: variant 20, sound 21, lying 22, relax 23, collar 24; the
    /// collar shows when the cat is tamed (TamableAnimal flags 18, bit 4).
    pub const CAT_COLLAR: u8 = 24;
    /// `Wolf.DATA_COLLAR_COLOR`: interested 20, collar 21, anger 22, variant 23 (tamed =
    /// TamableAnimal flags 18, bit 4).
    pub const WOLF_COLLAR: u8 = 21;
    /// `Shulker.DATA_COLOR_ID`: attach face 16, peek 17, colour byte 18 (16 = none).
    pub const SHULKER_COLOR: u8 = 18;
    /// `TropicalFish.DATA_ID_TYPE_VARIANT`: AbstractFish from-bucket 16, then the packed
    /// variant 17 (pattern `id & 0xFFFF`, base colour `id >> 16`, pattern colour `id >> 24`).
    pub const TROPICAL_FISH_VARIANT: u8 = 17;
    /// `TamableAnimal.DATA_FLAGS_ID` (tamed = bit 4, sitting = bit 1).
    pub const TAME_FLAGS: u8 = 18;
    /// Tamed bit of [`TAME_FLAGS`].
    pub const TAME_FLAG_TAMED: i8 = 4;
}

/// Entity metadata serializer ids for the `Holder<XVariant>` fields (the registration order
/// of `EntityDataSerializers`' static block; the protocol's metadata decoder keeps these as
/// `Other { type_id }`).
pub mod serializer_id {
    pub const CAT_VARIANT: i32 = 21;
    pub const COW_VARIANT: i32 = 23;
    pub const WOLF_VARIANT: i32 = 25;
    pub const FROG_VARIANT: i32 = 27;
    pub const PIG_VARIANT: i32 = 28;
    pub const CHICKEN_VARIANT: i32 = 30;
}

/// The model a variant's entry names (vanilla's `ModelAndTexture.model`, "normal" when the
/// field is absent — `ModelAndTexture.codec`'s `optionalFieldOf` default). Only pig, cow and
/// chicken variants carry a model; cat, wolf and frog always use their one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VariantModel {
    Normal,
    Cold,
    Warm,
}

impl VariantModel {
    /// The model string of a synced entry ("normal" is the codec's default when the field is
    /// absent). `None` for a model name we have no layer for (mods): the default look.
    pub fn parse(s: &str) -> Option<VariantModel> {
        match s {
            "normal" => Some(VariantModel::Normal),
            "cold" => Some(VariantModel::Cold),
            "warm" => Some(VariantModel::Warm),
            _ => None,
        }
    }
}

/// One resolved variant: the textures (asset ids, e.g. `minecraft:entity/pig/pig_cold`;
/// `haste-assets` maps them to `textures/<namespace>/<path>.png`) and the model type.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantAssets {
    /// The adult texture (`asset_id`; wolf's `assets.wild`).
    pub adult: Identifier,
    /// The baby texture (`baby_asset_id`; wolf's `baby_assets.wild`).
    pub baby: Identifier,
    /// Which temperature model layers the renderer bakes for this variant.
    pub model: VariantModel,
}

impl VariantAssets {
    /// One entry from its NBT fields (`asset_id`, optional `baby_asset_id` and `model`), with
    /// the asset ids exactly as the synced registry sent them. A frog has no baby form
    /// (tadpoles are their own entity), so its entries carry `asset_id` only and the baby
    /// falls back to the adult texture.
    pub fn from_assets(
        adult: &str,
        baby: Option<&str>,
        model: Option<&str>,
    ) -> Option<VariantAssets> {
        Some(VariantAssets {
            adult: Identifier::parse(adult).ok()?,
            baby: match baby {
                Some(b) => Identifier::parse(b).ok()?,
                None => Identifier::parse(adult).ok()?,
            },
            model: match model {
                Some(m) => VariantModel::parse(m)?,
                None => VariantModel::Normal,
            },
        })
    }
}
