//! wgpu render backend: chunk meshing (off-thread), atlas, sky/fog, culling, entity models, GUI batch drawing. Owner: WS3.
//!
//! See docs/INTERFACES.md for the contract this crate implements and AGENTS.md for the rules.
//!
//! Mesher/vertex packing derived from Pomme (GPL-3.0); see THIRD_PARTY.md.

pub mod mesher;
pub mod vertex;

pub use mesher::{mesh_section, MeshSnapshot, Mesher, SectionMesh};
pub use vertex::PackedVertex;
