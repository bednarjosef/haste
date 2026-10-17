//! The particle types and their sprites, packed into one atlas (vanilla `ParticleResources`
//! loading the particle jsons into sprite sets).

use anyhow::Result;
use haste_assets::{Assets, AtlasRegions};
use haste_core::Identifier;

/// The sprite list of one particle type: rectangles in the particle atlas, in file order.
pub(super) struct ParticleType {
    pub(super) name: Identifier,
    pub(super) sprites: Vec<[u16; 4]>,
}

/// Every particle type that has a particle json (name -> atlas sprite rectangles), plus the
/// packed atlas pixels. Built once from the registries' particle type ids; the game thread
/// resolves spawn names against it, the renderer uploads its pixels as one texture.
pub struct ParticleTypes {
    pub(super) types: Vec<ParticleType>,
    pub(super) by_name: ahash::HashMap<Identifier, u32>,
    /// Terrain-particle sprites per block state (vanilla `TerrainParticle`).
    pub(crate) blocks: Option<crate::block_particles::BlockParticleTable>,
    pub(super) pixels: Vec<u8>,
    pub(super) size: u32,
}

impl ParticleTypes {
    /// Packs the sprites of `ids` into one atlas, and with `states` also every block
    /// state's model `particle` texture (terrain particles: block breaking and mining
    /// cracks), so both draw from the one texture the renderer binds. Types without a
    /// definition or with no placed sprites contribute nothing.
    pub fn build(
        assets: &mut Assets,
        ids: &[Identifier],
        states: Option<&dyn haste_core::view::BlockStateTable>,
    ) -> Result<ParticleTypes> {
        // The particle json texture ids (`minecraft:generic_7` ->
        // `minecraft:particle/generic_7`), in file order per type.
        let mut definitions: Vec<(Identifier, Vec<Identifier>)> = Vec::new();
        let mut texture_ids: Vec<Identifier> = Vec::new();
        for id in ids {
            if let Some(definition) = assets.particle_definition(id)? {
                let mut sprites = Vec::with_capacity(definition.textures.len());
                for texture in &definition.textures {
                    sprites.push(crate::block_particles::particle_texture_id(texture));
                }
                texture_ids.extend(sprites.iter().cloned());
                definitions.push((id.clone(), sprites));
            }
        }
        // Block particle textures resolve per state (see `block_particles`).
        let (block_textures, block_ids) = match states {
            Some(states) => crate::block_particles::block_particle_textures(assets, states),
            None => (Vec::new(), Vec::new()),
        };
        texture_ids.extend(block_ids);
        texture_ids.sort();
        texture_ids.dedup();
        let atlas = haste_assets::AtlasBuilder::build(assets, &texture_ids, false)?;
        let mut types = Vec::new();
        for (id, sprites) in &definitions {
            let rects: Vec<[u16; 4]> = sprites
                .iter()
                .map(|t| atlas.region_of(&t.to_string()).pixel_rect.map(|v| v as u16))
                .collect();
            if !rects.is_empty() {
                types.push(ParticleType {
                    name: id.clone(),
                    sprites: rects,
                });
            }
        }
        let by_name = types
            .iter()
            .enumerate()
            .map(|(i, t)| (t.name.clone(), i as u32))
            .collect();
        let blocks = states.map(|states| {
            crate::block_particles::block_particle_table(states, block_textures, &atlas)
        });
        Ok(ParticleTypes {
            types,
            by_name,
            blocks,
            pixels: atlas.pixels().to_vec(),
            size: atlas.size(),
        })
    }

    /// Index of a particle type, `None` when it has no atlas sprites.
    pub(super) fn index(&self, name: &Identifier) -> Option<u32> {
        self.by_name.get(name).copied()
    }

    pub(super) fn sprites(&self, i: u32) -> &[[u16; 4]] {
        &self.types[i as usize].sprites
    }

    /// The packed atlas RGBA pixels and its side length (for the texture upload).
    pub fn atlas(&self) -> (&[u8], u32) {
        (&self.pixels, self.size)
    }
}
