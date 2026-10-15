//! wgpu render backend: chunk meshing (off-thread), atlas, sky/fog, culling, entity models, GUI batch drawing. Owner: renderer.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//!
//! Mesher/vertex packing derived from Pomme (GPL-3.0); see THIRD_PARTY.md.

pub mod animation;
pub mod arena;
pub mod blockentity;
pub mod entity;
pub mod fluid;
pub mod fog;
pub mod gui;
pub mod lightmap;
pub mod mesher;
pub mod occlusion;
pub mod overlay;
pub mod particles;
pub mod pipeline;
pub mod renderer;
pub mod sky;
pub mod timing;
pub mod tint;
pub mod vertex;
pub mod weather;

pub use blockentity::{
    chest_texture, kind_of, shulker_texture, xmas_textures, BlockEntityInstance, BlockEntityKind,
    ChestHalf, ChestMaterial, SignSideText, SignTextData,
};
pub use entity::{default_skin, EntityRenderer, FirstPerson, Nametag, TickPair};
pub use fog::{
    frame_fog, setup_fog, BiomeFog, DimensionFog, FogData, FogEnvironment, FogInputs, FogType,
};
pub use gui::{GuiRenderer, GuiStats, ScreenNametag};
pub use mesher::{mesh_section, MeshSnapshot, Mesher, SectionMesh, SnapshotSource};
pub use occlusion::{compute_visible_mask, section_visibility, Face, VisibilitySet};
pub use particles::{
    DustOption, ParticleRenderer, ParticleSpawn, ParticleTypes, Particles, MAX_PARTICLES,
};
pub use pipeline::{upload_atlas, upload_section, Gpu, SectionGpuMesh};
pub use renderer::{Camera, FrameInput, FrameStats, LightmapFrame, Renderer, SkyState, ViewBob};
pub use sky::{SkyColors, SkyKind};
pub use timing::FrameTimings;
pub use tint::{BiomeClimate, BiomeColors, BiomeSource, Colormaps, GrassModifier};
pub use vertex::PackedVertex;
pub use weather::{WeatherColumn, WeatherFrame};
