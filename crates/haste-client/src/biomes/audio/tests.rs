//! Tests for the `audio/music_volume` attribute: parsing from the synced registries and
//! the camera's layered resolution.

use std::sync::{Arc, OnceLock};

use glam::DVec3;
use haste_core::game::DimensionInfo;
use haste_core::{ChunkPos, Identifier};
use haste_data::GameData;
use haste_protocol::nbt::NbtCompound;
use haste_protocol::wire::Writer;
use haste_protocol::SyncedRegistries;
use haste_world::palette::{BiomeContainer, BlockContainer, WireWriter};
use haste_world::World;

use super::{
    biome_audio, camera_music_volume, dimension_music_volume, BiomeAudio,
    MUSIC_VOLUME_ATTRIBUTE_DEFAULT,
};

/// The NBT bytes of one registry entry, as the network codec writes them.
fn entry_bytes(c: &NbtCompound) -> bytes::Bytes {
    let mut w = Writer::new();
    w.nbt_compound(Some(c));
    w.into_inner().into()
}

#[test]
fn parses_the_biomes_music_volume() {
    let mut attrs = NbtCompound::new();
    attrs.put_float("minecraft:audio/music_volume", 0.0);
    let mut pale_garden = NbtCompound::new();
    pale_garden.put_compound("attributes", attrs);
    let mut regs = SyncedRegistries::default();
    regs.insert(
        Identifier::minecraft("worldgen/biome"),
        vec![
            haste_protocol::packets::configuration::clientbound::RegistryEntry {
                id: Identifier::minecraft("plains"),
                data: None,
            },
            haste_protocol::packets::configuration::clientbound::RegistryEntry {
                id: Identifier::minecraft("pale_garden"),
                data: Some(entry_bytes(&pale_garden)),
            },
        ],
    );
    // The pale garden carries 0; the plains entry carries nothing (the dimension value
    // shows through there).
    assert_eq!(
        biome_audio(&regs),
        vec![
            BiomeAudio { music_volume: None },
            BiomeAudio {
                music_volume: Some(0.0)
            },
        ]
    );
}

#[test]
fn dimension_value_defaults_without_an_entry() {
    let mut regs = SyncedRegistries::default();
    // No dimension types at all: the attribute's default.
    assert_eq!(
        dimension_music_volume(&regs, &DimensionInfo::overworld()),
        MUSIC_VOLUME_ATTRIBUTE_DEFAULT
    );
    // A dimension type whose attribute map carries the entry.
    let mut attrs = NbtCompound::new();
    attrs.put_float("minecraft:audio/music_volume", 0.25);
    let mut dim = NbtCompound::new();
    dim.put_compound("attributes", attrs);
    regs.insert(
        Identifier::minecraft("dimension_type"),
        vec![
            haste_protocol::packets::configuration::clientbound::RegistryEntry {
                id: Identifier::minecraft("overworld"),
                data: Some(entry_bytes(&dim)),
            },
        ],
    );
    assert_eq!(
        dimension_music_volume(&regs, &DimensionInfo::overworld()),
        0.25
    );
}

/// A minimal `GameData` (air only), so the world tests need no cache.
fn data() -> &'static Arc<GameData> {
    static D: OnceLock<Arc<GameData>> = OnceLock::new();
    D.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("haste-music-volume-test-{}", std::process::id()));
        let reports = dir.join("reports");
        std::fs::create_dir_all(&reports).unwrap();
        std::fs::write(
            reports.join("blocks.json"),
            r#"{"minecraft:air": {"states": [{"id": 0, "default": true}]}}"#,
        )
        .unwrap();
        std::fs::write(
            reports.join("registries.json"),
            r#"{"minecraft:block": {"entries": {"minecraft:air": {"protocol_id": 0}}}}"#,
        )
        .unwrap();
        Arc::new(GameData::load_from(&reports, None).unwrap())
    })
}

/// A chunk whose every section carries one biome (the test encoder
/// `encode_synthetic_chunk` builds always fills biome 0, so the sections are written
/// here with a chosen biome).
fn chunk_with_biome(
    pos: ChunkPos,
    biome: u32,
    dim: &DimensionInfo,
) -> haste_core::chunk::ChunkData {
    let mut w = WireWriter::new();
    for _ in 0..dim.section_count() {
        haste_world::wire::encode_section(
            &mut w,
            0,
            &BlockContainer::single(0),
            &BiomeContainer::single_biome(biome),
            4,
        );
    }
    haste_core::chunk::ChunkData {
        pos,
        heightmaps: Vec::new(),
        sections: bytes::Bytes::from(w.data),
        block_entities: Vec::new(),
        light: Default::default(),
    }
}

#[test]
fn camera_resolves_the_layered_attribute() {
    let dim = DimensionInfo::overworld();
    let mut world = World::new_with_biomes(dim.clone(), data().clone(), 16);
    world
        .load_chunk(&chunk_with_biome(ChunkPos::new(0, 0), 7, &dim))
        .unwrap();
    let mut table = vec![BiomeAudio { music_volume: None }; 16];
    // Biome 0 is the unloaded-cell fallback; biome 7 is the chunk's.
    table[0] = BiomeAudio {
        music_volume: Some(0.5),
    };
    table[7] = BiomeAudio {
        music_volume: Some(0.0),
    };
    // Inside the chunk: the biome's 0 wins over the dimension's 1.
    assert_eq!(
        camera_music_volume(&world, &table, DVec3::new(8.5, 70.0, 8.5), 1.0),
        0.0
    );
    // Outside every loaded chunk: the cell reads biome 0.
    assert_eq!(
        camera_music_volume(&world, &table, DVec3::new(-8.5, 70.0, 8.5), 1.0),
        0.5
    );
    // A biome without the entry keeps the dimension type's value.
    table[7] = BiomeAudio { music_volume: None };
    assert_eq!(
        camera_music_volume(&world, &table, DVec3::new(8.5, 70.0, 8.5), 0.25),
        0.25
    );
    // The attribute's unit range clamps the layered value.
    table[7] = BiomeAudio {
        music_volume: Some(2.0),
    };
    assert_eq!(
        camera_music_volume(&world, &table, DVec3::new(8.5, 70.0, 8.5), 1.0),
        1.0
    );
}

#[test]
fn unknown_biome_keeps_the_dimension_value() {
    let dim = DimensionInfo::overworld();
    let mut world = World::new_with_biomes(dim.clone(), data().clone(), 16);
    world
        .load_chunk(&chunk_with_biome(ChunkPos::new(0, 0), 3, &dim))
        .unwrap();
    // An empty synced table (every biome unknown, a modded registry): the dimension value.
    assert_eq!(
        camera_music_volume(&world, &[], DVec3::new(8.5, 70.0, 8.5), 0.25),
        0.25
    );
    assert_eq!(
        camera_music_volume(&world, &[], DVec3::new(8.5, 70.0, 8.5), 1.0),
        MUSIC_VOLUME_ATTRIBUTE_DEFAULT
    );
}

/// The quart math reaches the right cell through the whole chunk (the camera just inside
/// the chunk's high corner against a neighbour that carries another biome).
#[test]
fn camera_resolves_across_chunk_borders() {
    let dim = DimensionInfo::overworld();
    let mut world = World::new_with_biomes(dim.clone(), data().clone(), 16);
    world
        .load_chunk(&chunk_with_biome(ChunkPos::new(0, 0), 1, &dim))
        .unwrap();
    world
        .load_chunk(&chunk_with_biome(ChunkPos::new(1, 0), 2, &dim))
        .unwrap();
    let table = vec![
        BiomeAudio { music_volume: None },
        BiomeAudio {
            music_volume: Some(0.0),
        },
        BiomeAudio {
            music_volume: Some(1.0),
        },
    ];
    // x 15.9 is chunk (0,0) (biome 1); x 16.0 is chunk (1,0) (biome 2).
    assert_eq!(
        camera_music_volume(&world, &table, DVec3::new(15.9, 70.0, 8.5), 1.0),
        0.0
    );
    assert_eq!(
        camera_music_volume(&world, &table, DVec3::new(16.0, 70.0, 8.5), 1.0),
        1.0
    );
}
