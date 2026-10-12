//! wgpu render backend: chunk meshing (off-thread), atlas, sky/fog, culling, entity models, GUI batch drawing. Owner: renderer.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//!
//! Mesher/vertex packing derived from Pomme (GPL-3.0); see THIRD_PARTY.md.

pub mod arena;
pub mod entity;
pub mod gui;
pub mod mesher;
pub mod occlusion;
pub mod overlay;
pub mod pipeline;
pub mod renderer;
pub mod sky;
pub mod tint;
pub mod vertex;

pub use entity::{default_skin, EntityRenderer, FirstPerson, Nametag};
pub use gui::{GuiRenderer, GuiStats, ScreenNametag};
pub use mesher::{mesh_section, MeshSnapshot, Mesher, SectionMesh};
pub use occlusion::{compute_visible_mask, section_visibility, Face, VisibilitySet};
pub use pipeline::{upload_atlas, upload_section, Gpu, SectionGpuMesh};
pub use renderer::{Camera, FrameInput, FrameStats, Renderer, SkyState};
pub use sky::{SkyColors, SkyKind};
pub use tint::{BiomeClimate, BiomeColors, BiomeSource, Colormaps, GrassModifier};
pub use vertex::PackedVertex;
