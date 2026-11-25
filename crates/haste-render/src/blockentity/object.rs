// The object-model block entities' shared plumbing: the part builder (`ObjectPart`) and the
// emitter that draws vanilla's object models as they are. The renderers themselves (lectern
// and enchanting table books, bells, decorated pots, conduits) live in their own modules and
// submit through [`emit_object`].

use glam::{Mat3, Mat4, Vec3};
use haste_core::model::{Cube, CubeModel, ModelPart};
use haste_core::parts;

use crate::entity::{entity_shade, hurt_color, EntityVertex};

// -------------------------------------------------------------------------------------------
// Object-model block entities: the lectern and enchanting table books, bells, decorated pots
// and conduits (vanilla `LecternRenderer`, `EnchantTableRenderer`, `BellRenderer`,
// `DecoratedPotRenderer`, `ConduitRenderer`).
//
// Their layers (`BookModel.createBodyLayer`, `BellModel.createBodyLayer`,
// `DecoratedPotRenderer.createBaseLayer`/`createSidesLayer`, `ConduitRenderer.create*Layer`) are
// *object* models, not entity models: y up and world aligned (vanilla's block entity pose stack
// carries no y flip), the model's front at -z, one model unit = 1/16 block, and vanilla's own
// cube UV layout (the polygon at the layer's min-y end takes the texture's top slot).
// `emit_part` is the entity convention (y down), so these go through `emit_object`, which emits
// vanilla's polygons as they are — same corners, same texture rectangles, same winding fix-up —
// and the root matrix carries only the renderer's own translate/rotate plus the 1/16.

/// One part under construction: pivot, base rotation, and its cubes appended by
/// [`ObjectPart::cube`].
pub(super) struct ObjectPart {
    name: String,
    pivot: Vec3,
    rotation: Vec3,
    cubes: Vec<Cube>,
}

impl ObjectPart {
    pub(super) fn cube(mut self, origin: Vec3, size: Vec3, inflate: f32, uv: [u32; 2]) -> Self {
        self.cubes.push(Cube {
            origin,
            size,
            inflate,
            uv,
            mirror: false,
        });
        self
    }

    pub(super) fn done(self, children: Vec<ModelPart>) -> ModelPart {
        ModelPart {
            id: parts::intern(&self.name),
            name: self.name,
            pivot: self.pivot,
            rotation: self.rotation,
            visible: true,
            cubes: self.cubes,
            children,
        }
    }
}

pub(super) fn part(name: &str, pivot: Vec3, rotation: Vec3) -> ObjectPart {
    ObjectPart {
        name: name.into(),
        pivot,
        rotation,
        cubes: Vec::new(),
    }
}

pub(super) fn empty_root(children: Vec<ModelPart>) -> ModelPart {
    ModelPart {
        name: "root".into(),
        id: parts::ROOT,
        pivot: Vec3::ZERO,
        rotation: Vec3::ZERO,
        visible: true,
        cubes: Vec::new(),
        children,
    }
}

/// Poses one object-model block entity into `out`, one part at a time. `offset` is added
/// to every pivot (vanilla's `ModelPart.x/y/z` slide, which the book's pages use); `pose` holds
/// the per-frame part rotations, keyed by part name; `rect_of` resolves one texture rectangle
/// per part (the pot's sherd sides differ, everything else repeats the same one). The parts and
/// their names come from the model itself, so nothing is allocated per block entity.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_object(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    offset: Vec3,
    pose: &[(&'static str, Vec3)],
    root: Mat4,
    light: (u8, u8),
    rect_of: &mut dyn FnMut(&str) -> [u32; 4],
) {
    for part in &model.root.children {
        let rect = rect_of(&part.name);
        emit_object_part(out, model, part, pose, root, rect, offset, light);
    }
}

/// One part and its children, vanilla's `ModelPart.render` then `ModelPart.Cube.compile`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_object_part(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &[(&'static str, Vec3)],
    parent: Mat4,
    rect: [u32; 4],
    offset: Vec3,
    light: (u8, u8),
) {
    let over = pose.iter().find(|(n, _)| *n == part.name).map(|(_, r)| *r);
    let rot = part.rotation + over.unwrap_or(Vec3::ZERO);
    let m = parent
        * Mat4::from_translation(part.pivot + offset)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x);
    let normal_matrix = Mat3::from_mat4(m).inverse().transpose();
    let kx = rect[2] as f32 / model.tex_width.max(1) as f32;
    let ky = rect[3] as f32 / model.tex_height.max(1) as f32;
    let r16 = rect.map(|v| v as u16);
    for c in &part.cubes {
        let g = c.inflate;
        let (x0, y0, z0) = (c.origin.x - g, c.origin.y - g, c.origin.z - g);
        let (x1, y1, z1) = (
            c.origin.x + c.size.x + g,
            c.origin.y + c.size.y + g,
            c.origin.z + c.size.z + g,
        );
        let (u, v) = (c.uv[0] as i32 as f32, c.uv[1] as i32 as f32);
        let (w, h, d) = (c.size.x, c.size.y, c.size.z);
        // Vanilla `ModelPart.Cube`'s corner and UV layout (object convention, y up).
        let (t0, t1, t2, t3) = (
            Vec3::new(x0, y0, z0),
            Vec3::new(x1, y0, z0),
            Vec3::new(x1, y1, z0),
            Vec3::new(x0, y1, z0),
        );
        let (l0, l1, l2, l3) = (
            Vec3::new(x0, y0, z1),
            Vec3::new(x1, y0, z1),
            Vec3::new(x1, y1, z1),
            Vec3::new(x0, y1, z1),
        );
        let (u0, u1, u2, u22, u3, u4) = (
            u,
            u + d,
            u + d + w,
            u + d + 2.0 * w,
            u + d + w + d,
            u + d + w + d + w,
        );
        let (v0, v1, v2) = (v, v + d, v + d + h);
        // (corners in vanilla's order, uv rectangle, normal), one per direction.
        let faces: [([Vec3; 4], [f32; 4], Vec3); 6] = [
            ([l1, l0, t0, t1], [u1, v0, u2, v1], Vec3::NEG_Y),
            ([t2, t3, l3, l2], [u2, v1, u22, v0], Vec3::Y),
            ([t0, l0, l3, t3], [u0, v1, u1, v2], Vec3::NEG_X),
            ([t1, t0, t3, t2], [u1, v1, u2, v2], Vec3::NEG_Z),
            ([l1, t1, t2, l2], [u2, v1, u3, v2], Vec3::X),
            ([l0, l1, l2, l3], [u3, v1, u4, v2], Vec3::Z),
        ];
        for (corners, uvr, n) in faces {
            let p = corners.map(|c| m.transform_point3(c));
            let nw = (normal_matrix * n).normalize_or_zero();
            let s = entity_shade(nw);
            let [r, g, b] = hurt_color(s, 0.0);
            let color = [r, g, b, 1.0];
            // Vanilla's `ModelPart.Polygon` remap: the first and last corners take
            // (u1, v0)/(u1, v1), the middle two (u0, v0)/(u0, v1).
            let uv = [
                [rect[0] as f32 + uvr[2] * kx, rect[1] as f32 + uvr[1] * ky],
                [rect[0] as f32 + uvr[0] * kx, rect[1] as f32 + uvr[1] * ky],
                [rect[0] as f32 + uvr[0] * kx, rect[1] as f32 + uvr[3] * ky],
                [rect[0] as f32 + uvr[2] * kx, rect[1] as f32 + uvr[3] * ky],
            ];
            let ccw = (p[1] - p[0]).cross(p[2] - p[0]).dot(nw) >= 0.0;
            let order: [usize; 6] = if ccw {
                [0, 1, 2, 0, 2, 3]
            } else {
                [0, 2, 1, 0, 3, 2]
            };
            for i in order {
                out.push(EntityVertex {
                    pos: p[i].to_array(),
                    uv: uv[i],
                    color,
                    rect: r16,
                    light: [light.0 as u16, light.1 as u16, 0, 0],
                });
            }
        }
    }
    for child in &part.children {
        emit_object_part(out, model, child, pose, m, rect, offset, light);
    }
}

/// One model's part rotations for the shared pose, as name-rotation pairs. The object models
/// have a handful of parts, so a fixed array keeps the pose off the heap (per frame and per
/// block entity); the unused slots carry a name no part has.
pub(crate) type ObjectPose = [(&'static str, Vec3); 8];

/// The pose for `entries`, padded with names no part has.
pub(super) fn pose_of(entries: &[(&'static str, Vec3)]) -> ObjectPose {
    let mut out: ObjectPose = [("", Vec3::ZERO); 8];
    for (slot, e) in out.iter_mut().zip(entries) {
        *slot = *e;
    }
    out
}
