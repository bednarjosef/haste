// The GUI item lighting (`items_3d_lights`, `items_flat_lights`, `lambert_shade`,
// `transform_gui_normal`) is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/world/block/model.rs, reused under GPL-3.0; see THIRD_PARTY.md.

//! GUI items: slot geometry prepared from item models (vanilla `GuiItemRenderState` and the
//! item renderer's GUI path) and the GUI item lighting.

use std::sync::Arc;

use glam::{Mat3, Mat4, Quat, Vec3};
use haste_assets::{Assets, ItemGeometry, ItemModel};

use super::*;

/// A slot item ready to draw: quads in GUI pixels relative to the slot's top-left corner,
/// back to front.
pub(super) struct PreparedItem {
    pub(super) quads: Vec<PreparedQuad>,
}

pub(super) struct PreparedQuad {
    pub(super) pos: [[f32; 2]; 4],
    /// Face-local 0..1 texture coordinates.
    pub(super) uv: [[f32; 2]; 4],
    pub(super) tex: TexKey,
    pub(super) color: [u8; 4],
    /// The glint this quad draws when the stack has foil: its texture's sheet
    /// ([`crate::glint::item_glint_mode`]), resolved once with the model.
    pub(super) glint: crate::glint::GlintMode,
}

impl GuiRenderer {
    /// A slot item at (x, y); `foil` draws the enchantment glint on its quads.
    pub(super) fn item(
        &mut self,
        o: [f32; 2],
        item: u32,
        x: f32,
        y: f32,
        foil: haste_core::model::FoilType,
        assets: &mut Assets,
    ) {
        let prepared = match self.items.get(&item) {
            Some(p) => p.clone(),
            None => {
                let p = self
                    .item_names
                    .get(item as usize)
                    .cloned()
                    .and_then(|name| match assets.item_model(&name) {
                        Ok(model) => Some(Arc::new(prepare_item(&model))),
                        Err(e) => {
                            tracing::warn!("item model {name}: {e:#}");
                            None
                        }
                    });
                self.items.insert(item, p.clone());
                p
            }
        };
        let Some(prepared) = prepared else {
            // Unknown item: the missing texture, flat.
            let (rect, _) = self.lookup_ref(TexKeyRef::Key(&TexKey::Missing), assets);
            let r = rect.map(|v| v as f32);
            self.textured(
                o,
                [x, y, x + 16.0, y + 16.0],
                [r[0], r[1], r[0] + r[2], r[1] + r[3]],
                rect,
                [255; 4],
            );
            return;
        };
        for q in &prepared.quads {
            let (rect, _) = self.lookup_ref(TexKeyRef::Key(&q.tex), assets);
            let r = rect.map(|v| v as f32);
            let pos = q.pos.map(|p| [p[0] + x + o[0], p[1] + y + o[1]]);
            let uv = q.uv.map(|t| [r[0] + t[0] * r[2], r[1] + t[1] * r[3]]);
            let glint = if foil.has_foil() {
                q.glint
            } else {
                crate::glint::GlintMode::None
            };
            self.quad_glint(pos, uv, [q.color; 4], rect, glint as u32);
        }
    }
}
// ---- GUI item geometry and lighting ----

/// Pomme `items_3d_lights`: the two GUI light directions for side-lit (3D) items, in the
/// space of `transform_gui_normal`.
pub(super) fn items_3d_lights() -> (Vec3, Vec3) {
    let pose = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))
        * Mat4::from_rotation_y(1.082_104_1)
        * Mat4::from_rotation_x(3.237_585_8)
        * Mat4::from_rotation_y(-std::f32::consts::PI / 8.0)
        * Mat4::from_rotation_x(2.356_194_5);
    let t = |x: f32, y: f32, z: f32| pose.transform_vector3(Vec3::new(x, y, z).normalize());
    (t(0.2, 1.0, -0.7), t(-0.2, 1.0, 0.7))
}

/// Pomme `items_flat_lights`: the GUI light directions for front-lit (flat) items.
pub(super) fn items_flat_lights() -> (Vec3, Vec3) {
    let pose =
        Mat4::from_rotation_y(-std::f32::consts::PI / 8.0) * Mat4::from_rotation_x(2.356_194_5);
    let t = |x: f32, y: f32, z: f32| pose.transform_vector3(Vec3::new(x, y, z).normalize());
    (t(0.2, 1.0, -0.7), t(-0.2, 1.0, 0.7))
}

/// Pomme `lambert_shade`: two lights at 0.6 plus 0.4 ambient, capped at 1.
pub(super) fn lambert_shade(n: Vec3, l0: Vec3, l1: Vec3) -> f32 {
    ((n.dot(l0).max(0.0) + n.dot(l1).max(0.0)) * 0.6 + 0.4).min(1.0)
}

/// Display transform matrix: translate, rotate X then Y then Z, scale.
pub(crate) fn display_matrix(d: &haste_assets::DisplayTransform) -> Mat4 {
    Mat4::from_translation(Vec3::from(d.translation))
        * Mat4::from_rotation_x(d.rotation[0].to_radians())
        * Mat4::from_rotation_y(d.rotation[1].to_radians())
        * Mat4::from_rotation_z(d.rotation[2].to_radians())
        * Mat4::from_scale(Vec3::from(d.scale))
}

pub(crate) fn part_matrix(t: &Option<haste_assets::PartTransformation>) -> Mat4 {
    match t {
        None => Mat4::IDENTITY,
        Some(t) => {
            Mat4::from_translation(Vec3::from(t.translation))
                * Mat4::from_quat(Quat::from_array(t.left_rotation).normalize())
                * Mat4::from_scale(Vec3::from(t.scale))
                * Mat4::from_quat(Quat::from_array(t.right_rotation).normalize())
        }
    }
}

/// Builds the slot geometry of an item: model space (0..1 blocks) through the part
/// transformation, the `display.gui` transform, and the slot mapping (16 px, y down, centred at
/// 8, 8). Back faces are dropped and the rest sorted back to front.
pub(super) fn prepare_item(model: &ItemModel) -> PreparedItem {
    let mut quads: Vec<(f32, PreparedQuad)> = Vec::new();
    let slot = Mat4::from_translation(Vec3::new(8.0, 8.0, 0.0))
        * Mat4::from_scale(Vec3::new(16.0, -16.0, 16.0));
    for part in &model.parts {
        let local = display_matrix(&part.gui) * part_matrix(&part.transformation);
        let full = slot * local * Mat4::from_translation(Vec3::splat(-0.5));
        let normal_matrix = Mat3::from_mat4(Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0)) * local)
            .inverse()
            .transpose();
        let (l0, l1) = if part.gui_light_front {
            items_flat_lights()
        } else {
            items_3d_lights()
        };
        let tint = |index: Option<u32>| -> [u8; 4] {
            index
                .and_then(|i| part.tints.get(i as usize))
                .map(|c| rgba(*c | 0xFF00_0000))
                .unwrap_or([255; 4])
        };
        let mut push = |positions: [[f32; 3]; 4],
                        uv: [[f32; 2]; 4],
                        normal: Vec3,
                        tex: String,
                        tint: [u8; 4],
                        bias: f32| {
            let n = (normal_matrix * normal).normalize_or_zero();
            if n.z <= 1e-4 {
                return;
            }
            let shade = lambert_shade(n, l0, l1);
            let s = (shade * 255.0).round() as u8;
            let color = mul(tint, [s, s, s, 255]);
            let p = positions.map(|p| full.transform_point3(Vec3::from(p)));
            let depth = (p[0].z + p[1].z + p[2].z + p[3].z) / 4.0 + bias;
            quads.push((
                depth,
                PreparedQuad {
                    pos: p.map(|v| [v.x, v.y]),
                    uv,
                    glint: crate::glint::item_glint_mode(
                        haste_core::model::FoilType::Standard,
                        &tex,
                    ),
                    tex: TexKey::ItemTexture(tex),
                    color,
                },
            ));
        };
        match &part.geometry {
            ItemGeometry::None => {}
            ItemGeometry::Generated { layers } => {
                // The extruded sprite's front face and the 1/16-deep sides at its opaque
                // edges (`haste_assets`'s `generated_layer_quads`, vanilla's
                // `ItemModelGenerator`); the mirrored back face never shows in a slot.
                for (i, layer) in layers.iter().enumerate() {
                    for q in &layer.quads {
                        if q.direction == haste_assets::Direction::North {
                            continue;
                        }
                        let o = q.direction.offset();
                        push(
                            q.positions,
                            q.uvs,
                            Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                            layer.texture.clone(),
                            tint(q.tint_index),
                            i as f32 * 1e-3,
                        );
                    }
                }
            }
            ItemGeometry::Elements(list) => {
                for q in list {
                    let o = q.direction.offset();
                    push(
                        q.positions,
                        q.uvs,
                        Vec3::new(o[0] as f32, o[1] as f32, o[2] as f32),
                        q.texture.clone(),
                        tint(q.tint_index),
                        0.0,
                    );
                }
            }
        }
    }
    quads.sort_by(|a, b| a.0.total_cmp(&b.0));
    PreparedItem {
        quads: quads.into_iter().map(|(_, q)| q).collect(),
    }
}
