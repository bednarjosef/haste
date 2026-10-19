//! wgpu render backend: chunk meshing (off-thread), atlas, sky/fog, culling, entity models, GUI batch drawing. Owner: renderer.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//!
//! Mesher/vertex packing derived from Pomme (GPL-3.0); see THIRD_PARTY.md.

pub mod animation;
pub mod arena;
pub mod block_particles;
pub mod blockentity;
pub mod blur;
pub mod clouds;
pub mod entity;
pub mod fluid;
pub mod fluid_occlusion;
pub mod fog;
pub mod gui;
pub(crate) mod java_random;
pub mod lightmap;
pub mod mesher;
pub mod occlusion;
pub mod overlay;
pub mod panorama;
pub mod particles;
pub mod pipeline;
pub mod renderer;
pub mod section_slab;
pub mod sky;
pub mod stars;
pub mod timing;
pub mod tint;
pub mod vertex;
pub mod weather;

pub use block_particles::{BlockParticleSprite, BlockParticleTable};
pub use blockentity::{
    chest_texture, kind_of, shulker_texture, xmas_textures, BeamData, BeamSection, BlockEntityAnim,
    BlockEntityInstance, BlockEntityKind, ChestHalf, ChestMaterial, GatewayBeam, PistonData,
    PotSherds, SignSideText, SignTextData,
};
pub use entity::{EntityRenderer, FirstPerson, MapData, MapMarker, Nametag, TickPair};
pub use fog::{
    frame_fog, setup_fog, BiomeFog, DimensionFog, FogData, FogEnvironment, FogInputs, FogType,
};
pub use gui::{GuiRenderer, GuiStats, ScreenNametag};
pub use mesher::{mesh_section, MeshSnapshot, Mesher, SectionMesh, SnapshotSource};
pub use occlusion::{compute_visible_mask, section_visibility, Face, VisibilitySet};
pub use particles::{
    DustOption, ParticleRenderer, ParticleSnapshot, ParticleSnapshots, ParticleSpawn,
    ParticleTypes, ParticleView, Particles, MAX_PARTICLES,
};
pub use pipeline::{upload_atlas, upload_section, Gpu, SectionGpuMesh};
pub use renderer::SectionLoadState;
pub use renderer::{
    Camera, FrameInput, FrameStats, LightmapFrame, PremeshMode, Renderer, SkyState, ViewBob,
    WorkBudgetMode,
};
pub use sky::{SkyColors, SkyKind};
pub use timing::FrameTimings;
pub use tint::{BiomeClimate, BiomeColors, BiomeSource, Colormaps, GrassModifier};
pub use vertex::PackedVertex;
pub use weather::{WeatherColumn, WeatherFrame};
