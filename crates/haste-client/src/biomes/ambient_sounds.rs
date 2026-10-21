//! The `audio/ambient_sounds` environment attribute (vanilla
//! `EnvironmentAttributes.AMBIENT_SOUNDS`, value type `AmbientSounds`): the biome's
//! looping ambience, its mood sound (`AmbientMoodSettings`) and its random additions
//! (`AmbientAdditionsSettings`), which `BiomeAmbientSoundsHandler` plays.
//!
//! Registry-driven: every value comes from the synced `worldgen/biome` and
//! `dimension_type` entries the server sent, so a mod's biome brings its own sounds. The
//! attribute is positional and not interpolated (`AttributeType.ofNotInterpolated`), so,
//! like `audio/music_volume`, it reads the raw quart cell at the position (vanilla
//! `BiomeManager.getNoiseBiomeAtPosition`); vanilla's `EnvironmentAttributeSystem` layers
//! the dimension type's value first and a biome's value replaces it whole (the type has
//! no modifiers). The overworld's dimension type carries the cave mood; no timeline or
//! weather layer touches the attribute.

use glam::DVec3;
use haste_core::game::DimensionInfo;
use haste_core::{BiomeId, Identifier};
use haste_protocol::nbt::{NbtCompound, NbtTag};
use haste_protocol::wire::Reader;
use haste_protocol::SyncedRegistries;

use super::audio::noise_biome_at;

/// The attribute's key in an attribute map.
const AMBIENT_SOUNDS_KEY: &str = "minecraft:audio/ambient_sounds";

/// Vanilla `AmbientMoodSettings`: the mood sound, the delay its moodiness counts against,
/// how far from the player the light is sampled, and how far past the sampled block the
/// sound plays.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientMoodSettings {
    pub sound: String,
    pub tick_delay: i32,
    pub block_search_extent: i32,
    pub offset: f64,
}

/// Vanilla `AmbientAdditionsSettings`: a sound and its chance per tick.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientAdditionsSettings {
    pub sound: String,
    pub tick_chance: f64,
}

/// Vanilla `AmbientSounds`: the optional loop, the optional mood and the additions.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientSounds {
    pub loop_sound: Option<String>,
    pub mood: Option<AmbientMoodSettings>,
    pub additions: Vec<AmbientAdditionsSettings>,
}

impl AmbientSounds {
    /// `AmbientSounds.EMPTY`, the attribute's default.
    pub const EMPTY: AmbientSounds = AmbientSounds {
        loop_sound: None,
        mood: None,
        additions: Vec::new(),
    };
}

/// The attribute's layers for the current level: the dimension type's value and each
/// synced biome's override (index = biome registry id; `None` where the biome carries no
/// entry and the dimension value shows through).
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientSoundsLayers {
    pub dimension: AmbientSounds,
    pub biomes: Vec<Option<AmbientSounds>>,
}

impl Default for AmbientSoundsLayers {
    fn default() -> Self {
        Self {
            dimension: AmbientSounds::EMPTY,
            biomes: Vec::new(),
        }
    }
}

impl AmbientSoundsLayers {
    /// The value in `biome`: its override, else the dimension type's value (a biome id
    /// the synced table does not know keeps the dimension value).
    pub fn for_biome(&self, biome: BiomeId) -> &AmbientSounds {
        self.biomes
            .get(biome.0 as usize)
            .and_then(Option::as_ref)
            .unwrap_or(&self.dimension)
    }

    /// The value at `pos` (vanilla `EnvironmentAttributeSystem.getValue(AMBIENT_SOUNDS,
    /// pos)` with no interpolator): the biome of the raw quart cell there. No allocation.
    pub fn at(&self, world: &haste_world::World, pos: DVec3) -> &AmbientSounds {
        self.for_biome(noise_biome_at(world, pos))
    }
}

/// The synced biomes' `audio/ambient_sounds` overrides, index = biome registry id.
pub fn biome_ambient_sounds(regs: &SyncedRegistries) -> Vec<Option<AmbientSounds>> {
    let Some(entries) = regs.entries(&Identifier::minecraft("worldgen/biome")) else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|e| {
            e.data
                .as_ref()
                .and_then(|bytes| Reader::new(bytes).nbt_compound().ok().flatten())
                .and_then(|c| attribute_entry(&c))
        })
        .collect()
}

/// The dimension type's `audio/ambient_sounds` (the constant dimension layer):
/// `AmbientSounds.EMPTY` when the type carries none.
pub fn dimension_ambient_sounds(
    regs: &SyncedRegistries,
    dimension: &DimensionInfo,
) -> AmbientSounds {
    let reg = Identifier::minecraft("dimension_type");
    regs.index_of(&reg, &dimension.dimension_type)
        .and_then(|i| regs.entry_nbt(&reg, i))
        .and_then(|nbt| attribute_entry(&nbt))
        .unwrap_or(AmbientSounds::EMPTY)
}

/// Both layers for a level of `dimension`.
pub fn ambient_sounds_layers(
    regs: &SyncedRegistries,
    dimension: &DimensionInfo,
) -> AmbientSoundsLayers {
    AmbientSoundsLayers {
        dimension: dimension_ambient_sounds(regs, dimension),
        biomes: biome_ambient_sounds(regs),
    }
}

/// One registry entry's `attributes` map's ambient sounds, when present and readable.
fn attribute_entry(entry: &NbtCompound) -> Option<AmbientSounds> {
    let value = entry.get_compound("attributes")?.get(AMBIENT_SOUNDS_KEY)?;
    let parsed = ambient_sounds_from_nbt(value);
    if parsed.is_none() {
        tracing::warn!("unreadable {AMBIENT_SOUNDS_KEY} entry; ignored");
    }
    parsed
}

/// `AmbientSounds.CODEC` over network NBT: optional `loop`, optional `mood`, and
/// `additions` as one entry or a list of them (`ExtraCodecs.compactListCodec`). A
/// malformed field makes the whole value unreadable, as the codec fails as a whole.
pub fn ambient_sounds_from_nbt(tag: &NbtTag) -> Option<AmbientSounds> {
    let c = tag.extract_compound()?;
    let loop_sound = match c.get("loop") {
        Some(t) => Some(sound_event(t)?),
        None => None,
    };
    let mood = match c.get("mood") {
        Some(t) => Some(mood_from_nbt(t.extract_compound()?)?),
        None => None,
    };
    let additions = match c.get("additions") {
        None => Vec::new(),
        Some(NbtTag::List(list)) => list
            .iter()
            .map(|t| additions_from_nbt(t.extract_compound()?))
            .collect::<Option<Vec<_>>>()?,
        Some(t) => vec![additions_from_nbt(t.extract_compound()?)?],
    };
    Some(AmbientSounds {
        loop_sound,
        mood,
        additions,
    })
}

/// `AmbientMoodSettings.CODEC`: `sound`, `tick_delay`, `block_search_extent`, `offset`.
fn mood_from_nbt(c: &NbtCompound) -> Option<AmbientMoodSettings> {
    Some(AmbientMoodSettings {
        sound: sound_event(c.get("sound")?)?,
        tick_delay: int(c.get("tick_delay")?)?,
        block_search_extent: int(c.get("block_search_extent")?)?,
        offset: double(c.get("offset")?)?,
    })
}

/// `AmbientAdditionsSettings.CODEC`: `sound`, `tick_chance`.
fn additions_from_nbt(c: &NbtCompound) -> Option<AmbientAdditionsSettings> {
    Some(AmbientAdditionsSettings {
        sound: sound_event(c.get("sound")?)?,
        tick_chance: double(c.get("tick_chance")?)?,
    })
}

/// `SoundEvent.CODEC`: a registered event by id (a string), or an inline event
/// (`sound_id` plus an optional range). The name comes back namespaced, as `sounds.json`
/// keys are after parsing.
fn sound_event(tag: &NbtTag) -> Option<String> {
    let id = match tag {
        NbtTag::String(s) => &**s,
        NbtTag::Compound(c) => c.get_string("sound_id")?,
        _ => return None,
    };
    Some(if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    })
}

/// An integer field (`Codec.INT`), from any whole-number tag.
fn int(tag: &NbtTag) -> Option<i32> {
    match *tag {
        NbtTag::Byte(v) => Some(v.into()),
        NbtTag::Short(v) => Some(v.into()),
        NbtTag::Int(v) => Some(v),
        _ => None,
    }
}

/// A double field (`Codec.DOUBLE`), from any numeric tag.
fn double(tag: &NbtTag) -> Option<f64> {
    match *tag {
        NbtTag::Double(v) => Some(v),
        NbtTag::Float(v) => Some(v.into()),
        NbtTag::Byte(v) => Some(v.into()),
        NbtTag::Short(v) => Some(v.into()),
        NbtTag::Int(v) => Some(v.into()),
        NbtTag::Long(v) => Some(v as f64),
        _ => None,
    }
}

#[cfg(test)]
#[path = "ambient_sounds/tests.rs"]
mod tests;
