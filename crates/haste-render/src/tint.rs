//! Biome and block tint colours for the chunk mesher.
//!
//! Sources: minecraft.wiki "Color", "Biome", "Grass Block", "Leaves", "Redstone Dust",
//! "Melon Seeds" (observable behaviour only). Every value here is either data from the
//! server (the synced `minecraft:worldgen/biome` registry), pixels of the player's colormap
//! textures, or a constant the wiki documents.
//!
//! - Grass, foliage and dry foliage colours come from the biome's explicit colour when it has
//!   one, else from the 256x256 colormap at `x = (1 - t) * 255`, `y = (1 - d * t) * 255` with
//!   `t`, `d` the biome temperature and downfall clamped to 0..1.
//! - `dark_forest` grass: `((c & 0xFEFEFE) + 0x28340A) >> 1`.
//! - `swamp` grass: vanilla picks between `0x4C763C` and `0x6A7039` by a position noise.
//!   APPROXIMATE: the noise is not implemented; swamp grass is always `0x6A7039`.
//! - Biome blending: the per-block colour is the average of the colour over a
//!   `(2r + 1)^2` square of blocks around it at the same height (Video Settings "Biome Blend",
//!   default r = 2), integer average per channel. APPROXIMATE: biomes are looked up per
//!   4x4x4 cell without vanilla's per-block coordinate fuzzing (seed-hash zoom).

use haste_core::view::BlockStateTable;
use haste_core::{BiomeId, BlockPos, BlockStateId};

/// Default biome blend radius (vanilla "Biome Blend" 5x5).
pub const DEFAULT_BLEND_RADIUS: i32 = 2;

/// Grass colour modifier of a biome (`effects.grass_color_modifier`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GrassModifier {
    #[default]
    None,
    DarkForest,
    Swamp,
}

/// The colour-relevant part of one biome registry entry (plain data; the client converts the
/// synced registry NBT into this).
#[derive(Clone, Debug, PartialEq)]
pub struct BiomeClimate {
    pub temperature: f32,
    pub downfall: f32,
    /// 0xRRGGBB.
    pub water_color: u32,
    pub grass_color: Option<u32>,
    pub foliage_color: Option<u32>,
    pub dry_foliage_color: Option<u32>,
    pub grass_modifier: GrassModifier,
}

impl BiomeClimate {
    /// `minecraft:plains` (temperature 0.8, downfall 0.4, water 0x3F76E4): used when the
    /// server sent no biome registry, and for biome ids past its end.
    pub fn plains() -> BiomeClimate {
        BiomeClimate {
            temperature: 0.8,
            downfall: 0.4,
            water_color: 0x3F76E4,
            grass_color: None,
            foliage_color: None,
            dry_foliage_color: None,
            grass_modifier: GrassModifier::None,
        }
    }
}

/// The three 256x256 colormaps (`textures/colormap/{grass,foliage,dry_foliage}.png`) as
/// 0xRRGGBB.
pub struct Colormaps {
    grass: Vec<u32>,
    foliage: Vec<u32>,
    dry_foliage: Vec<u32>,
}

fn image_to_rgb(img: &image::RgbaImage) -> Vec<u32> {
    if img.width() != 256 || img.height() != 256 {
        tracing::warn!(
            "colormap is {}x{}, expected 256x256",
            img.width(),
            img.height()
        );
    }
    let mut out = vec![0xFF00FF; 256 * 256];
    for (x, y, p) in img.enumerate_pixels() {
        if x < 256 && y < 256 {
            out[(y * 256 + x) as usize] = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
        }
    }
    out
}

impl Colormaps {
    /// Loads the colormaps from the player's assets.
    pub fn load(assets: &mut haste_assets::Assets) -> anyhow::Result<Colormaps> {
        let mut load = |name: &str| -> anyhow::Result<Vec<u32>> {
            let id = haste_core::Identifier::parse(&format!("minecraft:colormap/{name}"))?;
            Ok(image_to_rgb(&assets.texture(&id)?))
        };
        Ok(Colormaps {
            grass: load("grass")?,
            foliage: load("foliage")?,
            dry_foliage: load("dry_foliage")?,
        })
    }

    /// Colormaps from raw 256*256 0xRRGGBB pixels (tests).
    pub fn from_pixels(grass: Vec<u32>, foliage: Vec<u32>, dry_foliage: Vec<u32>) -> Colormaps {
        Colormaps {
            grass,
            foliage,
            dry_foliage,
        }
    }

    /// Uniform colormaps (no assets): every lookup returns `c`.
    pub fn uniform(c: u32) -> Colormaps {
        Colormaps::from_pixels(vec![c; 65536], vec![c; 65536], vec![c; 65536])
    }
}

/// Colormap lookup by climate (minecraft.wiki "Color": the downfall axis is scaled by the
/// temperature, both clamped to 0..1).
pub fn colormap_lookup(map: &[u32], temperature: f32, downfall: f32) -> u32 {
    let t = temperature.clamp(0.0, 1.0);
    let d = downfall.clamp(0.0, 1.0) * t;
    let x = ((1.0 - t) * 255.0) as usize;
    let y = ((1.0 - d) * 255.0) as usize;
    map.get(y * 256 + x).copied().unwrap_or(0xFF00FF)
}

/// Which colour a tinted quad of a block takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TintKind {
    /// Untinted (white): blocks whose model has `tintindex` but no colour provider.
    None,
    Grass,
    Foliage,
    DryFoliage,
    Water,
    /// A biome-independent colour (0xRRGGBB).
    Fixed(u32),
}

/// Biome kinds in `BiomeColors::per_biome` order.
const GRASS: usize = 0;
const FOLIAGE: usize = 1;
const DRY_FOLIAGE: usize = 2;
const WATER: usize = 3;

/// Per-biome resolved colours plus the blend radius.
#[derive(Clone)]
pub struct BiomeColors {
    /// [grass, foliage, dry_foliage, water] per biome registry id.
    per_biome: Vec<[u32; 4]>,
    /// Colours for ids past the end of the table (plains).
    fallback: [u32; 4],
    radius: i32,
}

fn resolve(climate: &BiomeClimate, maps: &Colormaps) -> [u32; 4] {
    let (t, d) = (climate.temperature, climate.downfall);
    let base_grass = climate
        .grass_color
        .unwrap_or_else(|| colormap_lookup(&maps.grass, t, d));
    let grass = match climate.grass_modifier {
        GrassModifier::None => base_grass,
        GrassModifier::DarkForest => ((base_grass & 0xFEFEFE) + 0x28340A) >> 1,
        // APPROXIMATE: vanilla alternates with 0x4C763C by a position noise.
        GrassModifier::Swamp => 0x6A7039,
    };
    [
        grass,
        climate
            .foliage_color
            .unwrap_or_else(|| colormap_lookup(&maps.foliage, t, d)),
        climate
            .dry_foliage_color
            .unwrap_or_else(|| colormap_lookup(&maps.dry_foliage, t, d)),
        climate.water_color,
    ]
}

impl BiomeColors {
    /// Resolves every biome's colours (index = biome registry id). Blend radius
    /// `DEFAULT_BLEND_RADIUS`.
    pub fn new(biomes: &[BiomeClimate], maps: &Colormaps) -> BiomeColors {
        BiomeColors {
            per_biome: biomes.iter().map(|c| resolve(c, maps)).collect(),
            fallback: resolve(&BiomeClimate::plains(), maps),
            radius: DEFAULT_BLEND_RADIUS,
        }
    }

    /// Sets the blend radius (0 = no blending, 2 = vanilla default 5x5, max 7).
    pub fn with_radius(mut self, radius: i32) -> BiomeColors {
        self.radius = radius.clamp(0, 7);
        self
    }

    pub fn radius(&self) -> i32 {
        self.radius
    }

    /// Unblended colour of `kind` in `biome` (0xRRGGBB).
    pub fn color(&self, biome: BiomeId, kind: TintKind) -> u32 {
        let c = self
            .per_biome
            .get(biome.0 as usize)
            .unwrap_or(&self.fallback);
        match kind {
            TintKind::None => 0xFFFFFF,
            TintKind::Grass => c[GRASS],
            TintKind::Foliage => c[FOLIAGE],
            TintKind::DryFoliage => c[DRY_FOLIAGE],
            TintKind::Water => c[WATER],
            TintKind::Fixed(v) => v,
        }
    }

    /// Blended colour of `kind` at block `(x, z)` (any frame): the integer per-channel average
    /// of the colour over the `(2r+1)^2` square, `biome_at(x, z)` giving each block's biome.
    pub fn blended(
        &self,
        kind: TintKind,
        x: i32,
        z: i32,
        biome_at: impl Fn(i32, i32) -> BiomeId,
    ) -> u32 {
        match kind {
            TintKind::None => return 0xFFFFFF,
            TintKind::Fixed(v) => return v,
            _ => {}
        }
        let r = self.radius;
        let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
        for dz in -r..=r {
            for dx in -r..=r {
                let c = self.color(biome_at(x + dx, z + dz), kind);
                sr += (c >> 16) & 0xFF;
                sg += (c >> 8) & 0xFF;
                sb += c & 0xFF;
                n += 1;
            }
        }
        ((sr / n) << 16) | ((sg / n) << 8) | (sb / n)
    }
}

impl Default for BiomeColors {
    /// Plains everywhere with a uniform vanilla-default-like grass/foliage (no assets).
    fn default() -> Self {
        BiomeColors::new(&[], &Colormaps::uniform(0x91BD59))
    }
}

/// Per block state: which colour its tinted quads take (minecraft.wiki block pages).
pub struct TintTable {
    kinds: Vec<TintKind>,
}

/// Redstone dust colour by power 0..15 (minecraft.wiki "Redstone Dust": red rises from 0.3
/// at power 0 to 1.0 at 15; green and blue appear only at high power).
pub fn redstone_color(power: u8) -> u32 {
    let f = power.min(15) as f32 / 15.0;
    let r = f * 0.6 + if f > 0.0 { 0.4 } else { 0.3 };
    let g = (f * f * 0.7 - 0.5).clamp(0.0, 1.0);
    let b = (f * f * 0.6 - 0.7).clamp(0.0, 1.0);
    let c = |v: f32| ((v * 255.0) as u32).min(255);
    (c(r) << 16) | (c(g) << 8) | c(b)
}

/// Pumpkin/melon stem colour by age 0..7 (minecraft.wiki: green turning yellow-brown).
pub fn stem_color(age: u8) -> u32 {
    let a = age as u32;
    ((a * 32) << 16) | ((255 - a * 8) << 8) | (a * 4)
}

fn kind_for(states: &dyn BlockStateTable, state: BlockStateId) -> TintKind {
    let name = states.block_of(state).path();
    match name {
        "grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern" | "potted_fern"
        | "sugar_cane" | "bush" | "pink_petals" | "wildflowers" => TintKind::Grass,
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" | "vine" => TintKind::Foliage,
        "leaf_litter" => TintKind::DryFoliage,
        "water" | "bubble_column" | "water_cauldron" => TintKind::Water,
        // FoliageColor constants (minecraft.wiki "Leaves").
        "birch_leaves" => TintKind::Fixed(0x80A755),
        "spruce_leaves" => TintKind::Fixed(0x619961),
        "lily_pad" => TintKind::Fixed(0x208030),
        "attached_melon_stem" | "attached_pumpkin_stem" => TintKind::Fixed(0xE0C71C),
        "melon_stem" | "pumpkin_stem" => {
            let age = states
                .property_value(state, "age")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            TintKind::Fixed(stem_color(age))
        }
        "redstone_wire" => {
            let power = states
                .property_value(state, "power")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            TintKind::Fixed(redstone_color(power))
        }
        _ => TintKind::None,
    }
}

impl TintTable {
    pub fn new(states: &dyn BlockStateTable) -> TintTable {
        TintTable {
            kinds: (0..states.len() as u32)
                .map(|s| kind_for(states, BlockStateId(s)))
                .collect(),
        }
    }

    /// A table where every state is untinted (tests).
    pub fn empty() -> TintTable {
        TintTable { kinds: Vec::new() }
    }

    pub fn kind(&self, state: BlockStateId) -> TintKind {
        self.kinds
            .get(state.0 as usize)
            .copied()
            .unwrap_or(TintKind::None)
    }
}

/// Where the mesher reads biomes from at snapshot time (the world on the render thread).
pub trait BiomeSource {
    /// Biome at a block position (its 4x4x4 cell); `BiomeId(0)` where nothing is loaded.
    fn biome(&self, pos: BlockPos) -> BiomeId;
}

/// The biome cells a section's tints need: 4x4x4-block cells covering the section plus the
/// blend radius horizontally, captured with the block snapshot.
pub struct BiomeGrid {
    /// Cell coordinates of `cells[0]` (block >> 2).
    min: [i32; 3],
    dims: [i32; 3],
    cells: Vec<BiomeId>,
}

impl BiomeGrid {
    /// Captures the cells around section origin `origin` (block coords) for blend `radius`.
    pub fn capture(source: &dyn BiomeSource, origin: BlockPos, radius: i32) -> BiomeGrid {
        let min = [
            (origin.x - radius - 1) >> 2,
            origin.y >> 2,
            (origin.z - radius - 1) >> 2,
        ];
        let max = [
            (origin.x + 16 + radius) >> 2,
            (origin.y + 15) >> 2,
            (origin.z + 16 + radius) >> 2,
        ];
        let dims = [
            max[0] - min[0] + 1,
            max[1] - min[1] + 1,
            max[2] - min[2] + 1,
        ];
        let mut cells = Vec::with_capacity((dims[0] * dims[1] * dims[2]) as usize);
        for cy in 0..dims[1] {
            for cz in 0..dims[2] {
                for cx in 0..dims[0] {
                    cells.push(source.biome(BlockPos::new(
                        (min[0] + cx) << 2,
                        (min[1] + cy) << 2,
                        (min[2] + cz) << 2,
                    )));
                }
            }
        }
        BiomeGrid { min, dims, cells }
    }

    /// One biome everywhere (no world biomes known).
    pub fn uniform(biome: BiomeId) -> BiomeGrid {
        BiomeGrid {
            min: [0; 3],
            dims: [0; 3],
            cells: vec![biome],
        }
    }

    /// Biome at block (x, y, z); clamps to the captured cells.
    pub fn get(&self, x: i32, y: i32, z: i32) -> BiomeId {
        if self.dims[0] == 0 {
            return self.cells[0];
        }
        let c = |v: i32, a: usize| ((v >> 2) - self.min[a]).clamp(0, self.dims[a] - 1);
        let (cx, cy, cz) = (c(x, 0), c(y, 1), c(z, 2));
        self.cells[((cy * self.dims[2] + cz) * self.dims[0] + cx) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A colormap whose pixel at (x, y) encodes x in red and y in green.
    fn coordinate_map() -> Vec<u32> {
        (0..65536u32)
            .map(|i| ((i % 256) << 16) | ((i / 256) << 8))
            .collect()
    }

    #[test]
    fn colormap_lookup_uses_clamped_temperature_and_scaled_downfall() {
        let map = coordinate_map();
        // plains: t 0.8, d 0.4 -> x = (1 - 0.8f)*255 = 50.99 -> 50, y = (1 - 0.32)*255 = 173.4 -> 173.
        assert_eq!(colormap_lookup(&map, 0.8, 0.4), (50 << 16) | (173 << 8));
        // Desert (t 2.0, d 0): clamped to t 1 -> x 0, y 255.
        assert_eq!(colormap_lookup(&map, 2.0, 0.0), 255 << 8);
        // Frozen (t -0.5): t 0 -> x 255, d*t = 0 -> y 255.
        assert_eq!(colormap_lookup(&map, -0.5, 0.9), (255 << 16) | (255 << 8));
    }

    #[test]
    fn explicit_colors_and_modifiers() {
        let maps = Colormaps::from_pixels(coordinate_map(), vec![0x123456; 65536], vec![1; 65536]);
        let mut swamp = BiomeClimate::plains();
        swamp.foliage_color = Some(0x6A7039);
        swamp.water_color = 0x617B64;
        swamp.grass_modifier = GrassModifier::Swamp;
        let mut dark = BiomeClimate::plains();
        dark.grass_color = Some(0x507A32);
        dark.grass_modifier = GrassModifier::DarkForest;
        let colors = BiomeColors::new(&[BiomeClimate::plains(), swamp, dark], &maps);
        assert_eq!(
            colors.color(BiomeId(0), TintKind::Grass),
            (50 << 16) | (173 << 8)
        );
        assert_eq!(colors.color(BiomeId(0), TintKind::Foliage), 0x123456);
        assert_eq!(colors.color(BiomeId(1), TintKind::Foliage), 0x6A7039);
        assert_eq!(colors.color(BiomeId(1), TintKind::Water), 0x617B64);
        assert_eq!(colors.color(BiomeId(1), TintKind::Grass), 0x6A7039);
        assert_eq!(
            colors.color(BiomeId(2), TintKind::Grass),
            ((0x507A32 & 0xFEFEFE) + 0x28340A) >> 1
        );
        // Ids past the table fall back to plains; fixed colours ignore the biome.
        assert_eq!(colors.color(BiomeId(99), TintKind::Water), 0x3F76E4);
        assert_eq!(
            colors.color(BiomeId(1), TintKind::Fixed(0x80A755)),
            0x80A755
        );
    }

    #[test]
    fn blending_averages_a_square_per_channel() {
        let maps = Colormaps::uniform(0);
        let mut a = BiomeClimate::plains();
        a.water_color = 0x000000;
        let mut b = BiomeClimate::plains();
        b.water_color = 0xFF00FF;
        let colors = BiomeColors::new(&[a, b], &maps);
        // Biome 1 for x >= 0, biome 0 for x < 0.
        let at = |x: i32, _z: i32| BiomeId((x >= 0) as u32);
        // Radius 2 at x = 0: columns -2,-1 are biome 0, 0,1,2 biome 1 -> 3/5 of 255 = 153.
        assert_eq!(colors.blended(TintKind::Water, 0, 0, at), (153 << 16) | 153);
        // Far from the border: pure colour.
        assert_eq!(colors.blended(TintKind::Water, 10, 0, at), 0xFF00FF);
        assert_eq!(colors.blended(TintKind::Water, -10, 0, at), 0);
        // Radius 0: no blending.
        let colors = colors.with_radius(0);
        assert_eq!(colors.blended(TintKind::Water, 0, 0, at), 0xFF00FF);
        assert_eq!(colors.blended(TintKind::None, 0, 0, at), 0xFFFFFF);
    }

    #[test]
    fn biome_grid_covers_the_blend_square() {
        struct Stripes;
        impl BiomeSource for Stripes {
            fn biome(&self, pos: BlockPos) -> BiomeId {
                BiomeId(((pos.x >> 2) & 3) as u32 + 10 * ((pos.y >> 2) & 1) as u32)
            }
        }
        let origin = BlockPos::new(16, 64, -32);
        let grid = BiomeGrid::capture(&Stripes, origin, 2);
        for y in 64..80 {
            for z in -34..-14 {
                for x in 14..34 {
                    assert_eq!(grid.get(x, y, z), Stripes.biome(BlockPos::new(x, y, z)));
                }
            }
        }
    }

    #[test]
    fn redstone_and_stem_colors() {
        assert_eq!(redstone_color(0), 0x4C0000);
        assert_eq!(redstone_color(15), 0xFF3200);
        assert_eq!(stem_color(0), 0x00FF00);
        assert_eq!(stem_color(7), 0xE0C71C);
    }
}
