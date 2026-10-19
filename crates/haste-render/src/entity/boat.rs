//! The boat's water patch (vanilla `BoatRenderer.submitTypeAdditions`, the `water_patch` part
//! of `BoatModel.createWaterPatch`): a plate the size of the hull's bottom, laid at the
//! waterline, drawn depth-only so the translucent water inside a floating hull is culled
//! (vanilla's `RenderTypes.waterMask`, a colour-masked depth-writing render type). Haste's
//! `haste-entity` hands the patch over as an overlay model (never for a raft, never for a
//! boat under water); this module recognises it and emits its triangles into the mask buffer
//! instead of the coloured entity stream.

use super::*;

/// The marker texture `haste-entity`'s water patch overlay carries (its own namespace: it is
/// never sampled, and an accidental textured draw would show Haste's placeholder, not game
/// data).
pub(super) fn is_water_patch(model: &CubeModel) -> bool {
    model.texture.namespace() == "haste" && model.texture.path() == "boat/water_patch"
}

/// A position-only mask vertex, camera-relative like the entity vertices.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct MaskVertex {
    pos: [f32; 3],
}

impl MaskVertex {
    pub(super) const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            offset: 0,
            shader_location: 0,
            format: wgpu::VertexFormat::Float32x3,
        }],
    };
}

/// The mask vertices' own buffer (12-byte stride, unlike the entity vertices' 40).
pub(super) fn make_mask_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("boat-water-mask"),
        size: (capacity * std::mem::size_of::<MaskVertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl EntityRenderer {
    /// Emits a water patch overlay's triangles into this frame's mask buffer, position-only,
    /// through the same part matrices the coloured models use (`emit_part_tinted`'s pose
    /// maths without the colour, UV and light).
    pub(super) fn emit_water_mask(&mut self, model: &CubeModel, root: Mat4) {
        emit_mask_part(&mut self.mask_vertices, &model.root, root);
    }
}

/// Draws the frame's water masks (vanilla `water_mask`): depth write, no colour writes
/// (`RenderPipelines.WATER_MASK`'s masked colour target), no culling (the plate is seen from
/// above and below), before the translucent terrain so the water inside the hull fails the
/// depth test. Called by [`EntityRenderer::draw`]; the camera bind group must be set at
/// group 0.
pub(super) fn draw_masks(r: &EntityRenderer, pass: &mut wgpu::RenderPass<'_>) -> u32 {
    if r.mask_vertices.is_empty() {
        return 0;
    }
    pass.set_pipeline(&r.water_mask_pipeline);
    pass.set_vertex_buffer(0, r.mask_buffer.slice(..));
    pass.draw(0..r.mask_vertices.len() as u32, 0..1);
    1
}

/// One part of the mask model, posed exactly like `emit_part_tinted` poses a part (pivot,
/// then Z, Y, X rotations), its cubes' six faces as triangles. The pipeline draws without
/// culling, so one winding per face serves both sides.
fn emit_mask_part(out: &mut Vec<MaskVertex>, part: &ModelPart, parent: Mat4) {
    if !part.visible {
        return;
    }
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(part.rotation.z)
        * Mat4::from_rotation_y(part.rotation.y)
        * Mat4::from_rotation_x(part.rotation.x);
    for c in &part.cubes {
        let lo = c.origin - Vec3::splat(c.inflate);
        let hi = c.origin + c.size + Vec3::splat(c.inflate);
        // The eight corners (x0/x1, y0/y1, z0/z1).
        let mut p = [[0.0f32; 3]; 8];
        for (i, (x, y, z)) in [
            (lo.x, lo.y, lo.z),
            (hi.x, lo.y, lo.z),
            (lo.x, hi.y, lo.z),
            (hi.x, hi.y, lo.z),
            (lo.x, lo.y, hi.z),
            (hi.x, lo.y, hi.z),
            (lo.x, hi.y, hi.z),
            (hi.x, hi.y, hi.z),
        ]
        .into_iter()
        .enumerate()
        {
            let w = m.transform_point3(Vec3::new(x, y, z));
            p[i] = [w.x, w.y, w.z];
        }
        // Six faces, two triangles each, corner indices per face.
        for f in [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
        ] {
            for i in [f[0], f[1], f[2], f[0], f[2], f[3]] {
                out.push(MaskVertex { pos: p[i] });
            }
        }
    }
    for child in &part.children {
        emit_mask_part(out, child, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vanilla `BoatModel.createWaterPatch`: the plate is the bottom box laid flat (pivot
    /// (0, -3, 1), x rotation PI/2), so posed at the identity it spans the hull's 28 x 16
    /// model-unit footprint, level, from the boat origin's plane up to 3 units (the waterline
    /// the 0.375 lift puts it at).
    #[test]
    fn the_patch_lies_flat_at_the_waterline() {
        let patch = haste_entity::boat::water_patch_model();
        assert!(is_water_patch(&patch));
        let mut out = Vec::new();
        emit_mask_part(
            &mut out,
            &patch.root,
            Mat4::from_scale(Vec3::splat(1.0 / 16.0)),
        );
        // Six faces, two triangles each: 36 vertices.
        assert_eq!(out.len(), 36);
        // Every vertex sits within the flat band and the hull's footprint: the rotated box's
        // y (-9..7 around the -3 pivot, turned onto z) leaves y at -3..0 units.
        for v in &out {
            assert!(
                (-3.0 / 16.0 - 1.0e-6..=1.0e-6).contains(&v.pos[1]),
                "off the waterline: {:?}",
                v.pos
            );
            assert!(v.pos[0].abs() <= 14.0 / 16.0 + 1.0e-6, "{:?}", v.pos);
            assert!(v.pos[2].abs() <= 8.0 / 16.0 + 1.0e-6, "{:?}", v.pos);
        }
        // A model that is not the patch is not one.
        assert!(!is_water_patch(
            haste_entity::models::humanoid(false).as_ref()
        ));
    }
}
