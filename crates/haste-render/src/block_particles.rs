//! The client-spawned terrain particles (vanilla `TerrainParticle`): block breaking
//! and mining cracks, ported from vanilla 26.3's `ClientLevel.addDestroyBlockEffect`
//! (the 4x4x4 sprite grid a breaking block dissolves into) and
//! `ClientLevel.addBreakingParticles` (the one crack particle per mining tick on the
//! hit face), plus the block sprite table they draw from.
//!
//! The sprite of a block state is its model's `particle` texture (vanilla
//! `BlockStateModelSet.getParticleMaterial` -> the first model ref's particle
//! material), packed into the shared particle atlas by [`BlockParticleTable`], so
//! block particles draw in the same batched pass as `level_particles` types. The
//! tint follows vanilla `TerrainParticle`'s constructor: grey 0.6 times
//! `BlockTintSource.colorAsTerrainParticle` (the biome colour for grass/foliage/
//! water blocks, white where vanilla's source overrides it — grass_block's
//! particles are untinted — and the block below's biome for an upper double plant).
//!
//! Not modelled (same gaps as the atlas particles): block collisions (no `onGround`
//! drag), the `QuadParticleGroup` cap (the pool cap stands in), and animated block
//! sprites (water breaking draws the first frame, not the animation).

use std::collections::HashMap;

use haste_assets::Assets;
use haste_core::view::BlockStateTable;
use haste_core::{BlockStateId, Identifier};

use crate::tint::{BiomeColors, BiomeSource, TintKind, TintTable};

/// One block state's terrain-particle sprite and colour, resolved once at load.
#[derive(Clone, Copy, Debug)]
pub struct BlockParticleSprite {
    /// The `particle` texture's content rect in the shared particle atlas (texels).
    pub rect: [u16; 4],
    /// The colour `TerrainParticle` tints with (`colorAsTerrainParticle`).
    pub tint: TintKind,
    /// `BlockBehaviour.shouldSpawnTerrainParticles`: `minecraft:barrier` and
    /// `minecraft:structure_void` break silently (vanilla `noTerrainParticles`).
    pub spawn: bool,
    /// `doubleTallGrass`'s `colorInWorld` reads the biome one block below an upper
    /// half (`tall_grass`, `large_fern`).
    pub upper_half_tint_below: bool,
}

/// Per block state: the terrain-particle sprite (indexed by state id; `None` for
/// states without a model — air, or a blockstate json we could not read). Ids past
/// the table resolve to the placeholder state's entry, like the baked models.
pub struct BlockParticleTable {
    sprites: Vec<Option<BlockParticleSprite>>,
    placeholder: BlockStateId,
}

impl BlockParticleTable {
    /// The sprite of `state`, or the placeholder's for ids outside the table.
    pub fn sprite(&self, state: BlockStateId) -> Option<&BlockParticleSprite> {
        let id = state.0 as usize;
        let row = if id < self.sprites.len() {
            id
        } else {
            self.placeholder.0 as usize
        };
        self.sprites.get(row).and_then(|s| s.as_ref())
    }

    /// A one-state table mapping everything to `sprite` (tests).
    pub fn uniform(sprite: BlockParticleSprite) -> BlockParticleTable {
        BlockParticleTable {
            sprites: vec![Some(sprite)],
            placeholder: BlockStateId(0),
        }
    }
}

/// A particle json's texture id (`minecraft:generic_7`) as the sprite it names,
/// `minecraft:particle/generic_7` (the particle atlas's source directory; the path
/// is valid by construction: it was parsed as an `Identifier` already).
pub(crate) fn particle_texture_id(id: &Identifier) -> Identifier {
    Identifier::new(id.namespace(), &format!("particle/{}", id.path()))
}

/// Walk every block state's blockstate json to its model's `particle` texture
/// (vanilla `BlockStateModelSet.getParticleMaterial`: `SingleVariant` and
/// `WeightedVariants` use the matched variant's first model, `MultiPartModel` the
/// first case's). Returns the per-state texture id strings and the set of distinct
/// ids to pack (the caller builds one atlas over both particle types and blocks).
pub(crate) fn block_particle_textures(
    assets: &mut Assets,
    states: &dyn BlockStateTable,
) -> (Vec<Option<String>>, Vec<Identifier>) {
    // Blockstate definitions are read and parsed per block, not per state.
    let mut defs: HashMap<String, Option<haste_assets::BlockStateDefinition>> = HashMap::new();
    // Model -> its `particle` texture id (models are shared by many states).
    let mut models: HashMap<String, Option<String>> = HashMap::new();
    let mut textures: Vec<Option<String>> = Vec::with_capacity(states.len());
    let mut ids: Vec<Identifier> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for s in 0..states.len() as u32 {
        let state = BlockStateId(s);
        let block = states.block_of(state);
        defs.entry(block.to_string())
            .or_insert_with(|| assets.blockstate(block).ok());
        let Some(def) = defs.get(&block.to_string()).and_then(|d| d.as_ref()) else {
            textures.push(None);
            continue;
        };
        // Variants pick the matching entry's first model; multipart uses the first
        // case's model for the particle material regardless of `when`.
        let model = if def.variants.is_some() {
            def.model_refs_for(&states.properties(state))
                .first()
                .map(|r| r.model.clone())
        } else {
            def.all_model_refs().first().map(|r| r.model.clone())
        };
        let Some(model) = model else {
            textures.push(None);
            continue;
        };
        let texture = models
            .entry(model.clone())
            .or_insert_with(|| {
                let Ok(id) = Identifier::parse(&model) else {
                    return None;
                };
                assets.model(&id).ok().and_then(|m| {
                    m.textures.get("particle").map(|t| {
                        // Texture ids may omit the namespace (Pomme `normalize_texture_id`).
                        if t.contains(':') {
                            t.clone()
                        } else {
                            format!("minecraft:{t}")
                        }
                    })
                })
            })
            .clone();
        let Some(texture) = texture else {
            textures.push(None);
            continue;
        };
        if seen.insert(texture.clone()) {
            if let Ok(id) = Identifier::parse(&texture) {
                ids.push(id);
            }
        }
        textures.push(Some(texture));
    }
    (textures, ids)
}

/// The per-state particle tint kinds (vanilla `BlockTintSources`): the mesher's
/// table, with the states whose `colorAsTerrainParticle` differs fixed up —
/// `grassBlock` returns white for particles, and `pink_petals`/`wildflowers`'s
/// layer 0 is the blank (untinted) layer.
fn particle_tint_kinds(states: &dyn BlockStateTable) -> Vec<TintKind> {
    let table = TintTable::new(states);
    let mut kinds = Vec::with_capacity(states.len());
    for s in 0..states.len() as u32 {
        let state = BlockStateId(s);
        let block = states.block_of(state);
        let kind = table.kind(state);
        if block.namespace() == "minecraft"
            && matches!(block.path(), "grass_block" | "pink_petals" | "wildflowers")
        {
            kinds.push(TintKind::None);
        } else {
            kinds.push(kind);
        }
    }
    kinds
}

/// Builds the per-state block sprite table against an atlas that packed
/// `block_texture_ids` (from [`block_particle_textures`]).
pub(crate) fn block_particle_table(
    states: &dyn BlockStateTable,
    texture_ids: Vec<Option<String>>,
    atlas: &dyn haste_assets::AtlasRegions,
) -> BlockParticleTable {
    let kinds = particle_tint_kinds(states);
    let mut sprites = Vec::with_capacity(states.len());
    for (s, texture) in texture_ids.iter().enumerate() {
        let state = BlockStateId(s as u32);
        let block = states.block_of(state);
        let minecraft = block.namespace() == "minecraft";
        let Some(texture) = texture else {
            sprites.push(None);
            continue;
        };
        let rect = atlas.region_of(texture).pixel_rect.map(|v| v as u16);
        sprites.push(Some(BlockParticleSprite {
            rect,
            tint: kinds[s],
            // Vanilla `noTerrainParticles`: barrier and structure_void.
            spawn: !(minecraft && matches!(block.path(), "barrier" | "structure_void")),
            upper_half_tint_below: minecraft
                && matches!(block.path(), "tall_grass" | "large_fern")
                && states.property_value(state, "half") == Some("upper"),
        }));
    }
    BlockParticleTable {
        sprites,
        placeholder: states.placeholder_state(),
    }
}

/// The terrain-particle colour of `state` at `pos`: grey 0.6 times the tint source's
/// `colorAsTerrainParticle` (`TerrainParticle`'s constructor).
pub(crate) fn terrain_tint(
    sprite: &BlockParticleSprite,
    pos: haste_core::BlockPos,
    biomes: &dyn BiomeSource,
    colors: &BiomeColors,
) -> [f32; 3] {
    let sample = |p: haste_core::BlockPos| {
        colors.blended(sprite.tint, p.x, p.z, |x, z| {
            biomes.biome(haste_core::BlockPos::new(x, p.y, z))
        })
    };
    let pos = if sprite.upper_half_tint_below {
        // doubleTallGrass reads the block below an upper half.
        haste_core::BlockPos::new(pos.x, pos.y - 1, pos.z)
    } else {
        pos
    };
    let c = sample(pos);
    [
        0.6 * ((c >> 16 & 0xFF) as f32 / 255.0),
        0.6 * ((c >> 8 & 0xFF) as f32 / 255.0),
        0.6 * ((c & 0xFF) as f32 / 255.0),
    ]
}
