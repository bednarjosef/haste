//! Particle definitions and the particle atlas: `assets/<ns>/particles/<name>.json`
//! lists the sprites a particle type is drawn from (animated over the list, or one
//! picked at random); the atlas packs every referenced `textures/particle/*.png`
//! into one image. Block/item particle types have no json (they are drawn from the
//! block or item texture instead) and resolve to `None`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Deserialize;

use haste_core::Identifier;

use crate::assets::asset_key;
use crate::atlas::AtlasRegion;
use crate::Assets;

/// `particles/<name>.json`: the sprites a particle type is drawn from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParticleDefinition {
    /// Texture ids (`minecraft:generic_7`) in file order; a multi-entry list
    /// animates over the sprites or picks one at random.
    pub textures: Vec<Identifier>,
}

impl Assets {
    /// `particles/<id>.json` of a particle type, or `None` when it has none
    /// (block/item particle types and unknown names).
    pub fn particle_definition(&mut self, id: &Identifier) -> Result<Option<ParticleDefinition>> {
        let key = asset_key(id, "particles", ".json");
        let Some(bytes) = self.try_read_asset(&key)? else {
            return Ok(None);
        };
        let file: ParticleDefinitionFile =
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {key}"))?;
        let mut textures = Vec::with_capacity(file.textures.len());
        for entry in &file.textures {
            textures.push(Identifier::parse(entry)?);
        }
        Ok(Some(ParticleDefinition { textures }))
    }
}

#[derive(Deserialize)]
struct ParticleDefinitionFile {
    textures: Vec<String>,
}

/// A particle json's texture id (`minecraft:generic_7`) as the sprite it names,
/// `minecraft:particle/generic_7` (the particle atlas's source directory). The
/// path is valid by construction: it was parsed as an `Identifier` already.
fn particle_texture_id(id: &Identifier) -> Identifier {
    Identifier::new(id.namespace(), &format!("particle/{}", id.path()))
}

/// Every particle sprite in one atlas: the packer is the block atlas's shelf
/// packer reused without mips (particles render at native size).
pub struct ParticleAtlas {
    builder: crate::atlas::AtlasBuilder,
    /// Particle type -> one region per entry of its definition's texture list.
    sprites: HashMap<String, Vec<AtlasRegion>>,
}

impl ParticleAtlas {
    /// Packs every texture referenced by the given particle types' definitions
    /// (types without a definition contribute nothing). Missing textures become
    /// the missing tile, as in the block atlas.
    pub fn build(assets: &mut Assets, particle_ids: &[Identifier]) -> Result<ParticleAtlas> {
        let mut definitions = Vec::with_capacity(particle_ids.len());
        let mut texture_ids: Vec<Identifier> = Vec::new();
        for id in particle_ids {
            if let Some(definition) = assets.particle_definition(id)? {
                for texture in &definition.textures {
                    texture_ids.push(particle_texture_id(texture));
                }
                definitions.push((id.clone(), definition));
            }
        }
        texture_ids.sort();
        texture_ids.dedup();
        let builder = crate::atlas::AtlasBuilder::build(assets, &texture_ids, false)?;
        let sprites = definitions
            .into_iter()
            .map(|(id, definition)| {
                let regions = definition
                    .textures
                    .iter()
                    .filter_map(|texture| {
                        let key = particle_texture_id(texture).to_string();
                        (builder.has(&key)).then(|| builder.region(&key))
                    })
                    .collect();
                (id.to_string(), regions)
            })
            .collect();
        Ok(ParticleAtlas { builder, sprites })
    }

    /// The atlas region of a particle texture id as written in a definition
    /// (`minecraft:flame`'s `minecraft:generic_7`); `None` when the atlas does
    /// not pack it. Textures that failed to load resolve to the missing tile.
    pub fn region_of(&self, texture_id: &Identifier) -> Option<AtlasRegion> {
        let key = particle_texture_id(texture_id).to_string();
        (self.builder.has(&key)).then(|| self.builder.region(&key))
    }

    /// The regions of a particle type's sprite list, in file order; empty for
    /// types without a definition.
    pub fn sprites_of(&self, particle: &Identifier) -> &[AtlasRegion] {
        self.sprites
            .get(&particle.to_string())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The packed atlas RGBA pixels (no mip chain).
    pub fn pixels(&self) -> &[u8] {
        self.builder.pixels()
    }

    /// Atlas side length in texels (a power of two).
    pub fn size(&self) -> u32 {
        self.builder.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A jar with a hand-written particle json and its two 4x4 sprites, plus the
    /// two entries `Assets::open` requires.
    fn test_jar(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("client.jar");
        let file = std::fs::File::create(&path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let entry = |zip: &mut zip::ZipWriter<std::fs::File>, name: &str, bytes: Vec<u8>| {
            use std::io::Write;
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
            zip.start_file(name, opts).unwrap();
            zip.write_all(&bytes).unwrap();
        };
        entry(
            &mut zip,
            "assets/minecraft/blockstates/stone.json",
            b"{}".to_vec(),
        );
        entry(&mut zip, "assets/minecraft/lang/en_us.json", b"{}".to_vec());
        entry(
            &mut zip,
            "assets/minecraft/particles/test.json",
            br#"{"textures": ["minecraft:generic_0", "minecraft:other"]}"#.to_vec(),
        );
        for name in ["generic_0", "other"] {
            let mut png = Vec::new();
            image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 100, 50, 255]))
                .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();
            entry(
                &mut zip,
                &format!("assets/minecraft/textures/particle/{name}.png"),
                png,
            );
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn parses_a_particle_json_and_none_without_a_file() {
        let dir = std::env::temp_dir().join("haste-assets-particle-test");
        std::fs::create_dir_all(&dir).unwrap();
        let jar = test_jar(&dir);
        let mut assets = Assets::open(&jar, &dir, "unused").unwrap();

        let definition = assets
            .particle_definition(&Identifier::minecraft("test"))
            .unwrap()
            .expect("test.json exists");
        assert_eq!(
            definition.textures,
            [
                Identifier::parse("minecraft:generic_0").unwrap(),
                Identifier::parse("minecraft:other").unwrap()
            ]
        );

        // No file for this particle type (block/item particles have none either).
        assert!(assets
            .particle_definition(&Identifier::minecraft("block"))
            .unwrap()
            .is_none());

        // The referenced sprites pack into one atlas, one region per list entry.
        let atlas = ParticleAtlas::build(&mut assets, &[Identifier::minecraft("test")]).unwrap();
        let sprites = atlas.sprites_of(&Identifier::minecraft("test"));
        assert_eq!(sprites.len(), 2);
        assert_ne!(sprites[0], sprites[1]);
        for sprite in sprites {
            assert_eq!((sprite.width(), sprite.height()), (4, 4));
            assert!(!atlas.pixels().is_empty());
            assert!(atlas.size() >= 16);
        }
        let texture = Identifier::parse("minecraft:generic_0").unwrap();
        assert_eq!(atlas.region_of(&texture), Some(sprites[0]));
        assert_eq!(
            atlas.region_of(&Identifier::parse("minecraft:other").unwrap()),
            Some(sprites[1])
        );
        // A texture no definition references is not in the atlas.
        assert_eq!(
            atlas.region_of(&Identifier::parse("minecraft:absent").unwrap()),
            None
        );
        // A particle type without a definition has no sprites.
        assert!(atlas.sprites_of(&Identifier::minecraft("block")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "needs the client jar and datagen reports in the cache"]
    fn jar_particle_atlas_covers_vanilla_types() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let mut assets =
            Assets::open(&jar, &haste_core::paths::cache_dir().join("assets"), "26.3").unwrap();
        // Every `minecraft:particle_type` entry from the data generator's report.
        let registries: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(haste_core::paths::datagen_reports().join("registries.json"))
                .unwrap(),
        )
        .unwrap();
        let mut particle_ids: Vec<(u32, Identifier)> = registries["minecraft:particle_type"]
            ["entries"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, entry)| {
                (
                    entry["protocol_id"].as_u64().unwrap() as u32,
                    Identifier::parse(name).unwrap(),
                )
            })
            .collect();
        particle_ids.sort_by_key(|(id, _)| *id);
        let particle_ids: Vec<Identifier> = particle_ids.into_iter().map(|(_, id)| id).collect();

        let mut with_definition = 0;
        for id in &particle_ids {
            if assets.particle_definition(id).unwrap().is_some() {
                with_definition += 1;
            }
        }
        let atlas = ParticleAtlas::build(&mut assets, &particle_ids).unwrap();
        eprintln!(
            "{} particle types, {} with a definition, atlas {}x{}",
            particle_ids.len(),
            with_definition,
            atlas.size(),
            atlas.size()
        );
        assert!(with_definition > 0);
        // Sprites are either placed or the missing tile; both are real regions.
        for name in ["flame", "smoke", "heart"] {
            let particle = Identifier::minecraft(name);
            let sprites = atlas.sprites_of(&particle);
            assert!(
                !sprites.is_empty(),
                "minecraft:{name} has no particle definition"
            );
            assert!(
                sprites.iter().any(|sprite| sprite.width() > 0),
                "minecraft:{name} has no placed sprites"
            );
        }
    }
}
