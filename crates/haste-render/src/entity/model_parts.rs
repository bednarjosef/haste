//! Cube-model posing: part matrices, model extents and the per-part vertex emitter
//! (vanilla's `ModelPart`/`LivingEntityRenderer` pose plumbing).

use super::*;

/// Highest point of a model above its feet, in model units (y is down in model space).
pub(crate) fn model_top(part: &ModelPart, parent_y: f32) -> f32 {
    let y = parent_y + part.pivot.y;
    let own = part
        .cubes
        .iter()
        .map(|c| -(y + c.origin.y - c.inflate))
        .fold(0.0f32, f32::max);
    part.children
        .iter()
        .map(|c| model_top(c, y))
        .fold(own, f32::max)
}

/// A model's render size and camera-relative half-extents in blocks, for vanilla's
/// `Entity.shouldRenderAtSqrDistance` (`boundingBox.getSize()`) and the frustum test
/// (`getBoundingBoxForCulling`, the model's static bounds: like vanilla's AABB it does
/// not track animation). Model space is y-down, 1/16 block per unit; `emit_part` scales
/// by 1/16 with x/y mirrored, so extents are symmetric and only the scale matters.
pub(crate) fn model_extents(part: &ModelPart) -> (f32, f32) {
    fn walk(part: &ModelPart, ox: f32, oy: f32, oz: f32, lo: &mut Vec3, hi: &mut Vec3) {
        let (px, py, pz) = (ox + part.pivot.x, oy + part.pivot.y, oz + part.pivot.z);
        for c in &part.cubes {
            let (x0, y0, z0) = (px + c.origin.x, py + c.origin.y, pz + c.origin.z);
            let (x1, y1, z1) = (x0 + c.size.x, y0 + c.size.y, z0 + c.size.z);
            let i = c.inflate;
            *lo = lo.min(Vec3::new(x0 - i, y0 - i, z0 - i));
            *hi = hi.max(Vec3::new(x1 + i, y1 + i, z1 + i));
        }
        for child in &part.children {
            walk(child, px, py, pz, lo, hi);
        }
    }
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    walk(part, 0.0, 0.0, 0.0, &mut lo, &mut hi);
    let (w, h, d) = (
        (hi.x - lo.x).abs(),
        (hi.y - lo.y).abs(),
        (hi.z - lo.z).abs(),
    );
    // `AABB.getSize()` is the diagonal's length; model space is 1/16 block, so / 16.
    let size = ((w * w + h * h + d * d).sqrt() / 16.0).max(1.0);
    let half = Vec3::new(w, h, d) / 16.0 / 2.0;
    (size, half.length())
}

/// Conservative AABB-in-frustum test: the sphere around the entity (centre `rel` in
/// camera-relative blocks, radius `r`) against the 6 planes (Gribb/Hartmann, the same
/// plane form the terrain cull uses). The sphere contains the model's extents.
pub(crate) fn aabb_in_frustum(planes: &[glam::Vec4; 6], rel: glam::DVec3, r: f32) -> bool {
    let c = rel.as_vec3();
    planes
        .iter()
        .all(|p| p.x * c.x + p.y * c.y + p.z * c.z + p.w >= -r)
}

pub(crate) fn pose_rotation(pose: &PoseOverrides, id: PartId) -> Option<Vec3> {
    pose.rotation_of(id)
}

/// The accumulated model matrix of a named part, the same transform `emit_part` gives it
/// (`parent * translate(pivot) * R(z) R(y) R(x)`, pose overrides added to the base rotation), so
/// a held item attaches to the animated arm (vanilla `ArmedModel.translateToHand`). `None` when
/// the model has no such part.
pub(crate) fn part_matrix(
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    id: PartId,
) -> Option<Mat4> {
    let rot = part.rotation + pose_rotation(pose, part.id).unwrap_or(Vec3::ZERO);
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x);
    if part.id == id {
        return Some(m);
    }
    part.children
        .iter()
        .find_map(|c| part_matrix(c, pose, m, id))
}

/// Child pivots are relative to the parent (vanilla `PartPose`); rotation is Z * Y * X around
/// the pivot, pose overrides add to the base rotation. `hurt` (0..1) tints the part red;
/// `tint` multiplies the colour per channel (dyed overlay layers).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_part(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    rect: [u32; 4],
    light: (u8, u8),
    hurt: f32,
) {
    emit_part_tinted(out, model, part, pose, parent, rect, light, hurt, [1.0; 3]);
}

/// [`emit_part`] with a colour multiplier (vanilla's `submitModel` `color` parameter): armour
/// dye. The tint multiplies the shade/hurt colour.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_part_tinted(
    out: &mut Vec<EntityVertex>,
    model: &CubeModel,
    part: &ModelPart,
    pose: &PoseOverrides,
    parent: Mat4,
    rect: [u32; 4],
    light: (u8, u8),
    hurt: f32,
    tint: [f32; 3],
) {
    if !part.visible || pose.is_hidden(part.id) {
        return;
    }
    let rot = part.rotation + pose_rotation(pose, part.id).unwrap_or(Vec3::ZERO);
    let m = parent
        * Mat4::from_translation(part.pivot)
        * Mat4::from_rotation_z(rot.z)
        * Mat4::from_rotation_y(rot.y)
        * Mat4::from_rotation_x(rot.x);
    let normal_matrix = Mat3::from_mat4(m).inverse().transpose();
    let kx = rect[2] as f32 / model.tex_width.max(1) as f32;
    let ky = rect[3] as f32 / model.tex_height.max(1) as f32;
    let r16 = rect.map(|v| v as u16);
    for c in &part.cubes {
        let lo = c.origin - Vec3::splat(c.inflate);
        let hi = c.origin + c.size + Vec3::splat(c.inflate);
        let (u, v) = (c.uv[0] as f32, c.uv[1] as f32);
        let (w, h, d) = (c.size.x.floor(), c.size.y.floor(), c.size.z.floor());
        // Faces: corners (model space) in order top-left, bottom-left, bottom-right, top-right
        // as seen on the texture, the texture rectangle (u0, v0, u1, v1) and outward normal.
        // Model space: y down, the model's front is -z, its right side is -x.
        let (x0, y0, z0) = (lo.x, lo.y, lo.z);
        let (x1, y1, z1) = (hi.x, hi.y, hi.z);
        let mut faces: [([Vec3; 4], [f32; 4], Vec3); 6] = [
            // north (front): u grows with x, v with y
            (
                [
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x1, y1, z0),
                    Vec3::new(x1, y0, z0),
                ],
                [u + d, v + d, u + d + w, v + d + h],
                Vec3::NEG_Z,
            ),
            // south (back): u grows toward -x
            (
                [
                    Vec3::new(x1, y0, z1),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x0, y0, z1),
                ],
                [u + 2.0 * d + w, v + d, u + 2.0 * (d + w), v + d + h],
                Vec3::Z,
            ),
            // west (-x, the model's right side): u grows toward the front
            (
                [
                    Vec3::new(x0, y0, z1),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x0, y0, z0),
                ],
                [u, v + d, u + d, v + d + h],
                Vec3::NEG_X,
            ),
            // east (+x, the left side): u grows toward the back
            (
                [
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x1, y1, z0),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x1, y0, z1),
                ],
                [u + d + w, v + d, u + 2.0 * d + w, v + d + h],
                Vec3::X,
            ),
            // up (y0 is the top): v grows toward the front
            (
                [
                    Vec3::new(x0, y0, z1),
                    Vec3::new(x0, y0, z0),
                    Vec3::new(x1, y0, z0),
                    Vec3::new(x1, y0, z1),
                ],
                [u + d, v, u + d + w, v + d],
                Vec3::NEG_Y,
            ),
            // down
            (
                [
                    Vec3::new(x0, y1, z0),
                    Vec3::new(x0, y1, z1),
                    Vec3::new(x1, y1, z1),
                    Vec3::new(x1, y1, z0),
                ],
                [u + d + w, v, u + d + 2.0 * w, v + d],
                Vec3::Y,
            ),
        ];
        if c.mirror {
            // Mirrored cubes swap the side textures and flip every face horizontally.
            let west = faces[2].1;
            faces[2].1 = faces[3].1;
            faces[3].1 = west;
            for f in &mut faces {
                f.1.swap(0, 2);
            }
        }
        for (corners, uvr, n) in faces {
            let p = corners.map(|c| m.transform_point3(c));
            let nw = (normal_matrix * n).normalize_or_zero();
            let s = entity_shade(nw);
            let [r, g, b] = hurt_color(s, hurt);
            let color = [r * tint[0], g * tint[1], b * tint[2], 1.0];
            let uv = [
                [uvr[0], uvr[1]],
                [uvr[0], uvr[3]],
                [uvr[2], uvr[3]],
                [uvr[2], uvr[1]],
            ]
            .map(|t| [rect[0] as f32 + t[0] * kx, rect[1] as f32 + t[1] * ky]);
            // Wind counter-clockwise seen from outside, whatever the transform did.
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
        emit_part_tinted(out, model, child, pose, m, rect, light, hurt, tint);
    }
}
