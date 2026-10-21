//! The table row type and its constructors (see `registry`).

/// One table row: adult and baby layer ids (`minecraft:pig#main`), textures (jar paths under
/// `assets/minecraft/textures/`, without the prefix), and the layers' root scales.
pub(super) struct Entry {
    pub(super) layer: &'static str,
    pub(super) texture: &'static str,
    pub(super) root_scale: f32,
    /// The baby layer/texture when vanilla renders babies with their own `*_BABY` layer;
    /// `None` when the adult layer is used for both.
    pub(super) baby: Option<(&'static str, &'static str)>,
}

pub(super) const fn row(layer: &'static str, texture: &'static str) -> Entry {
    Entry {
        layer,
        texture,
        root_scale: 1.0,
        baby: None,
    }
}

pub(super) const fn row_baby(
    layer: &'static str,
    texture: &'static str,
    baby_layer: &'static str,
    baby_texture: &'static str,
) -> Entry {
    Entry {
        layer,
        texture,
        root_scale: 1.0,
        baby: Some((baby_layer, baby_texture)),
    }
}

pub(super) const fn row_scaled(
    layer: &'static str,
    texture: &'static str,
    root_scale: f32,
) -> Entry {
    Entry {
        layer,
        texture,
        root_scale,
        baby: None,
    }
}
