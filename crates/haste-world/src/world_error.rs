//! World errors.

#[derive(thiserror::Error, Debug)]
pub enum WorldError {
    #[error("chunk decode failed at {pos:?}: {source}")]
    Decode {
        pos: haste_core::ChunkPos,
        #[source]
        source: crate::palette::PaletteError,
    },
    #[error("palette error: {0}")]
    Palette(#[from] crate::palette::PaletteError),
}
