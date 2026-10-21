//! Tests for the `audio/ambient_sounds` attribute: the network NBT forms and the layering.

use haste_core::game::DimensionInfo;
use haste_core::{BiomeId, Identifier};
use haste_protocol::nbt::{NbtCompound, NbtTag};
use haste_protocol::packets::configuration::clientbound::RegistryEntry;
use haste_protocol::wire::Writer;
use haste_protocol::SyncedRegistries;

use super::*;

/// The NBT bytes of one registry entry, as the network codec writes them.
fn entry_bytes(c: &NbtCompound) -> bytes::Bytes {
    let mut w = Writer::new();
    w.nbt_compound(Some(c));
    w.into_inner().into()
}

/// The overworld dimension type's value in the data pack: the cave mood only.
fn cave_mood() -> NbtCompound {
    let mut mood = NbtCompound::new();
    mood.put_string("sound", "minecraft:ambient.cave".into());
    mood.put_int("tick_delay", 6000);
    mood.put_int("block_search_extent", 8);
    mood.put_double("offset", 2.0);
    let mut value = NbtCompound::new();
    value.put_compound("mood", mood);
    value
}

/// A nether biome's value: loop, mood and a single additions entry (the compact form).
fn basalt_deltas() -> NbtCompound {
    let mut value = cave_mood();
    value.put_string("loop", "minecraft:ambient.basalt_deltas.loop".into());
    let mut additions = NbtCompound::new();
    additions.put_string("sound", "minecraft:ambient.basalt_deltas.additions".into());
    additions.put_double("tick_chance", 0.0111);
    value.put_compound("additions", additions);
    value
}

fn with_attribute(value: NbtCompound) -> NbtCompound {
    let mut attrs = NbtCompound::new();
    attrs.put_compound(AMBIENT_SOUNDS_KEY, value);
    let mut entry = NbtCompound::new();
    entry.put_compound("attributes", attrs);
    entry
}

fn mood(sound: &str) -> AmbientMoodSettings {
    AmbientMoodSettings {
        sound: sound.into(),
        tick_delay: 6000,
        block_search_extent: 8,
        offset: 2.0,
    }
}

#[test]
fn parses_loop_mood_and_compact_additions() {
    let parsed = ambient_sounds_from_nbt(&NbtTag::Compound(basalt_deltas())).unwrap();
    assert_eq!(
        parsed,
        AmbientSounds {
            loop_sound: Some("minecraft:ambient.basalt_deltas.loop".into()),
            mood: Some(mood("minecraft:ambient.cave")),
            additions: vec![AmbientAdditionsSettings {
                sound: "minecraft:ambient.basalt_deltas.additions".into(),
                tick_chance: 0.0111,
            }],
        }
    );
}

#[test]
fn parses_additions_lists_and_inline_events() {
    // A list of additions, and an inline sound event (`sound_id`) without a namespace.
    let mut inline = NbtCompound::new();
    inline.put_string("sound_id", "ambient.custom".into());
    let mut one = NbtCompound::new();
    one.put_compound("sound", inline);
    one.put_double("tick_chance", 0.5);
    let mut two = NbtCompound::new();
    two.put_string("sound", "mod:wind".into());
    two.put_float("tick_chance", 0.25);
    let mut value = NbtCompound::new();
    value.put_list(
        "additions",
        vec![NbtTag::Compound(one), NbtTag::Compound(two)],
    );
    let parsed = ambient_sounds_from_nbt(&NbtTag::Compound(value)).unwrap();
    assert_eq!(parsed.loop_sound, None);
    assert_eq!(parsed.mood, None);
    assert_eq!(
        parsed.additions,
        vec![
            AmbientAdditionsSettings {
                sound: "minecraft:ambient.custom".into(),
                tick_chance: 0.5,
            },
            AmbientAdditionsSettings {
                sound: "mod:wind".into(),
                tick_chance: 0.25,
            },
        ]
    );
    // An empty compound is `AmbientSounds.EMPTY`; a mood missing a field is unreadable.
    assert_eq!(
        ambient_sounds_from_nbt(&NbtTag::Compound(NbtCompound::new())),
        Some(AmbientSounds::EMPTY)
    );
    let mut broken = cave_mood();
    let mut short = NbtCompound::new();
    short.put_string("sound", "minecraft:ambient.cave".into());
    broken.put_compound("mood", short);
    assert_eq!(ambient_sounds_from_nbt(&NbtTag::Compound(broken)), None);
}

#[test]
fn layers_the_biome_over_the_dimension_type() {
    let mut regs = SyncedRegistries::default();
    regs.insert(
        Identifier::minecraft("worldgen/biome"),
        vec![
            RegistryEntry {
                id: Identifier::minecraft("plains"),
                data: Some(entry_bytes(&NbtCompound::new())),
            },
            RegistryEntry {
                id: Identifier::minecraft("basalt_deltas"),
                data: Some(entry_bytes(&with_attribute(basalt_deltas()))),
            },
        ],
    );
    regs.insert(
        Identifier::minecraft("dimension_type"),
        vec![RegistryEntry {
            id: Identifier::minecraft("overworld"),
            data: Some(entry_bytes(&with_attribute(cave_mood()))),
        }],
    );
    let layers = ambient_sounds_layers(&regs, &DimensionInfo::overworld());
    assert_eq!(layers.dimension.mood, Some(mood("minecraft:ambient.cave")));
    // Plains carries no entry: the dimension's cave mood shows through.
    assert_eq!(layers.for_biome(BiomeId(0)), &layers.dimension);
    // The basalt deltas' value replaces the dimension's whole.
    assert_eq!(
        layers.for_biome(BiomeId(1)).loop_sound.as_deref(),
        Some("minecraft:ambient.basalt_deltas.loop")
    );
    // An id the synced table does not know (a modded registry) keeps the dimension value.
    assert_eq!(layers.for_biome(BiomeId(99)), &layers.dimension);
    // A dimension type without the entry is `AmbientSounds.EMPTY`.
    let mut nether = DimensionInfo::overworld();
    nether.dimension_type = Identifier::minecraft("the_nether");
    assert_eq!(
        dimension_ambient_sounds(&regs, &nether),
        AmbientSounds::EMPTY
    );
}
