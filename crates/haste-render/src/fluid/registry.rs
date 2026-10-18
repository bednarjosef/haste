//! The fluid registry and the per-block-state fluid table: which fluid a block state holds,
//! its rendering properties (sprites, tint, overlay, layer) and how fluids treat it as a
//! neighbour.

use haste_assets::{Direction, RenderLayer};
use haste_core::view::BlockStateTable;
use haste_core::{BlockPos, BlockStateId};

use crate::fluid_occlusion::Occluder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fluid {
    Water,
    Lava,
}

/// How a fluid is tinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidTint {
    /// No tint (lava).
    None,
    /// Biome water colour blend (water).
    BiomeWater,
}

/// A fluid's rendering properties (the `Fluids` registry values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidDef {
    pub tint: FluidTint,
    /// Render layer the fluid draws in.
    pub layer: RenderLayer,
    /// Whether sides facing glass/leaves use the water overlay sprite.
    pub overlay: bool,
    pub still: u16,
    pub flow: u16,
}

/// Registry of fluids: fluid id -> rendering properties. Water and lava are
/// registered by [`Fluids::vanilla`]; mods add their own fluids here.
#[derive(Clone, Copy, Debug)]
pub struct Fluids {
    water: FluidDef,
    lava: FluidDef,
}

impl Fluids {
    /// The two vanilla fluids, with their atlas sprites.
    pub fn vanilla(sprites: &FluidSprites) -> Fluids {
        Fluids {
            water: FluidDef {
                tint: FluidTint::BiomeWater,
                layer: RenderLayer::Translucent,
                overlay: true,
                still: sprites.water_still,
                flow: sprites.water_flow,
            },
            lava: FluidDef {
                tint: FluidTint::None,
                layer: RenderLayer::Solid,
                overlay: false,
                still: sprites.lava_still,
                flow: sprites.lava_flow,
            },
        }
    }

    /// Rendering properties of a fluid.
    pub fn def(&self, fluid: Fluid) -> FluidDef {
        match fluid {
            Fluid::Water => self.water,
            Fluid::Lava => self.lava,
        }
    }
}

impl Default for Fluids {
    fn default() -> Fluids {
        Fluids::vanilla(&FluidSprites::default())
    }
}

/// Decides which fluid a block state holds (resolved once per state at load).
///
/// The generic vanilla rules are: any state with `waterlogged=true` holds
/// water; the liquid blocks (`water`/`lava`) use their `level` property for
/// the height. Blocks that always hold a fluid source (kelp, seagrass, bubble
/// column, ...) are registered in [`FluidRules::vanilla`]; mods add more with
/// [`FluidRules::register_block`].
#[derive(Clone, Debug)]
pub struct FluidRules {
    /// Block path -> fluid, for blocks that always hold a fluid source.
    sources: std::collections::HashMap<&'static str, Fluid>,
}

impl FluidRules {
    /// The vanilla rules: the always-water plants.
    pub fn vanilla() -> FluidRules {
        let mut rules = FluidRules {
            sources: std::collections::HashMap::new(),
        };
        for block in [
            "kelp",
            "kelp_plant",
            "seagrass",
            "tall_seagrass",
            "bubble_column",
        ] {
            rules.register_block(block, Fluid::Water);
        }
        rules
    }

    /// Register a block that always holds a fluid source (mods).
    pub fn register_block(&mut self, block: &'static str, fluid: Fluid) {
        self.sources.insert(block, fluid);
    }

    fn fluid_of(&self, name: &str) -> Option<Fluid> {
        self.sources.get(name).copied()
    }
}

impl Default for FluidRules {
    fn default() -> FluidRules {
        FluidRules::vanilla()
    }
}

/// The fluid a block state holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FluidState {
    pub fluid: Fluid,
    /// Surface height of the block alone, 0..=8/9.
    pub height: f32,
}

/// Neighbour classes the fluid mesher needs per block state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Class {
    /// Water sides facing it use the overlay sprite (glass, leaves).
    overlay: bool,
}

/// Atlas sprite indices of the fluid textures.
#[derive(Clone, Copy, Debug, Default)]
pub struct FluidSprites {
    pub water_still: u16,
    pub water_flow: u16,
    pub water_overlay: u16,
    pub lava_still: u16,
    pub lava_flow: u16,
}

impl FluidSprites {
    pub fn from_atlas(atlas: &dyn haste_assets::AtlasRegions) -> FluidSprites {
        let s = |name: &str| atlas.region_of(name).sprite as u16;
        FluidSprites {
            water_still: s("minecraft:block/water_still"),
            water_flow: s("minecraft:block/water_flow"),
            water_overlay: s("minecraft:block/water_overlay"),
            lava_still: s("minecraft:block/lava_still"),
            lava_flow: s("minecraft:block/lava_flow"),
        }
    }
}

/// Per block state: its fluid and how fluids treat it as a neighbour.
pub struct FluidTable {
    fluids: Vec<Option<FluidState>>,
    classes: Vec<Class>,
    /// Fluid rendering properties (sprites, tint, overlay, layer).
    pub defs: Fluids,
    pub sprites: FluidSprites,
}

pub(super) fn level_height(level: u8) -> f32 {
    if level == 0 || level >= 8 {
        8.0 / 9.0
    } else {
        (8 - level) as f32 / 9.0
    }
}

impl FluidTable {
    pub fn new(
        states: &dyn BlockStateTable,
        defs: &Fluids,
        rules: &FluidRules,
        sprites: FluidSprites,
    ) -> FluidTable {
        let n = states.len();
        let mut fluids = Vec::with_capacity(n);
        let mut classes = Vec::with_capacity(n);
        for s in 0..n as u32 {
            let state = BlockStateId(s);
            let name = states.block_of(state).path();
            let fluid = if states.property_value(state, "waterlogged") == Some("true") {
                Some((Fluid::Water, 0))
            } else if let Some(level) = states
                .property_value(state, "level")
                .and_then(|v| v.parse::<u8>().ok())
            {
                match name {
                    "water" => Some((Fluid::Water, level)),
                    "lava" => Some((Fluid::Lava, level)),
                    _ => None,
                }
            } else {
                rules.fluid_of(name).map(|f| (f, 0))
            };
            fluids.push(fluid.map(|(fluid, level)| FluidState {
                fluid,
                height: level_height(level),
            }));
            classes.push(Class {
                overlay: name.ends_with("glass")
                    || name.ends_with("glass_pane")
                    || name.ends_with("leaves"),
            });
        }
        FluidTable {
            fluids,
            classes,
            defs: *defs,
            sprites,
        }
    }

    /// A table with no fluids (tests, before the block table is known).
    pub fn empty() -> FluidTable {
        FluidTable {
            fluids: Vec::new(),
            classes: Vec::new(),
            defs: Fluids::default(),
            sprites: FluidSprites::default(),
        }
    }

    /// A table from explicit per-state fluids (tests).
    pub fn from_fluids(fluids: Vec<Option<FluidState>>) -> FluidTable {
        let classes = vec![Class::default(); fluids.len()];
        FluidTable {
            fluids,
            classes,
            defs: Fluids::default(),
            sprites: FluidSprites::default(),
        }
    }

    pub fn fluid(&self, state: BlockStateId) -> Option<FluidState> {
        self.fluids.get(state.0 as usize).copied().flatten()
    }

    pub(super) fn overlay(&self, state: BlockStateId) -> bool {
        self.classes
            .get(state.0 as usize)
            .is_some_and(|c| c.overlay)
    }
}

/// What the fluid mesher reads around a block (the mesh snapshot, or a test grid).
pub trait FluidWorld {
    fn state(&self, p: BlockPos) -> BlockStateId;
    /// Full opaque cube (`BlockState.isSolidRender`).
    fn opaque(&self, p: BlockPos) -> bool;
    /// Ignored by corner averaging (vanilla: blocks motion).
    fn solid(&self, p: BlockPos) -> bool;
    /// The neighbour's occlusion shape on the face it shares with a fluid whose face `dir`
    /// points at it (`BlockState.getFaceOcclusionShape(dir.getOpposite())`). The default
    /// knows only full cubes; the mesher's view answers from `fluid/occlusion.rs`'s
    /// per-state table (slabs, stairs and other partial shapes as their boxes).
    fn face_occluder(&self, p: BlockPos, dir: Direction) -> Occluder<'_> {
        let _ = dir;
        if self.opaque(p) {
            Occluder::Full
        } else {
            Occluder::Empty
        }
    }
}
