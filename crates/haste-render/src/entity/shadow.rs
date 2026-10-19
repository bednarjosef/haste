//! The entity shadow pass (`EntityRenderer.extractShadow`): round shadow discs on the full
//! block tops below entities, fading with distance, height and block brightness.

use super::*;

use haste_core::view::CollisionShapes;
use std::sync::OnceLock;

/// Whether the entity is eligible for a shadow at all: invisible entities never cast
/// (`extractShadow` skips them), and a landed falling block is hidden entirely
/// (`FallingBlockRenderer.shouldRender` skips the whole entity, its shadow included).
/// Everything else casts with its renderer's `shadowRadius` — the `radius <= 0` gate below
/// then draws nothing for the radius-0 types (armour stands, item frames, paintings, the
/// bolt), exactly vanilla's per-renderer `state.shadowRadius`.
fn shadow_eligible(e: &EntityInstance) -> bool {
    !e.invisible
        && !matches!(
            e.decoration,
            Some(Decoration::FallingBlock(FallingBlockDecoration {
                landed: true,
                ..
            }))
        )
}

/// The vertex alpha that darkens the sRGB main target like vanilla's gamma-space blend
/// darkens its plain UNORM target. Vanilla's `ENTITY_SHADOW` (`BlendFunction.TRANSLUCENT`)
/// blends the target's sRGB bytes, so a piece alpha `a` removes exactly `a` of them; Haste's
/// main pass target is an sRGB format, whose blend multiplies linear light instead — the
/// stored bytes come out as `1 - (1 - a)^(1/2.2)` of darkening, roughly half of vanilla's.
/// Pre-distorting `a -> 1 - (1 - a)^2.2` makes the linear blend land on vanilla's bytes
/// (within the sRGB curve's small linear toe).
pub fn srgb_shadow_alpha(alpha: f32) -> f32 {
    1.0 - (1.0 - alpha).powf(2.2)
}

impl EntityRenderer {
    /// Vanilla `EntityRenderer.extractShadow`: the round `misc/shadow.png` quads on the full
    /// top faces below each entity. For every entity the shadow is a `shadowRadius` (capped at
    /// 32) disc, its power fading with camera distance (`1 - distSq/256`) times the renderer's
    /// shadow strength, and with height above the block (`extractShadowPiece`); the alpha also
    /// scales with the block's raw brightness through `Lightmap.getBrightness`. Skipped for
    /// invisible entities and when the entity-shadows option is off. `sky_darken` is
    /// `Level.getSkyDarken` (0..15) and `ambient_light` the dimension's `ambientLight`.
    ///
    /// Batched into one reused vertex buffer, no per-frame allocation.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_shadows(
        &mut self,
        entities: &[EntityInstance],
        skip: Option<EntityId>,
        camera: glam::DVec3,
        world: &dyn BlockView,
        shapes: &haste_data::BlockShapes,
        sky_darken: i32,
        ambient_light: f32,
        entity_shadows: bool,
        assets: &mut Assets,
    ) {
        self.shadow_vertices.clear();
        if !entity_shadows {
            return;
        }
        static SHADOW_TEXTURE: OnceLock<Identifier> = OnceLock::new();
        let shadow_id = SHADOW_TEXTURE.get_or_init(|| Identifier::minecraft("misc/shadow"));
        let rect = self.texture_rect(shadow_id, assets);
        for e in entities {
            if Some(e.id) == skip || !shadow_eligible(e) {
                continue;
            }
            let radius = e.shadow_radius.min(32.0);
            if radius <= 0.0 {
                continue;
            }
            let dist_sq = (e.pos - camera).length_squared();
            let pow = (1.0 - dist_sq / 256.0) as f32 * e.shadow_strength;
            if pow <= 0.0 {
                continue;
            }
            let x0 = (e.pos.x - radius as f64).floor() as i32;
            let x1 = (e.pos.x + radius as f64).floor() as i32;
            let z0 = (e.pos.z - radius as f64).floor() as i32;
            let z1 = (e.pos.z + radius as f64).floor() as i32;
            let depth = (pow / 0.5 - 1.0).min(radius);
            let y0 = (e.pos.y - depth as f64).floor() as i32;
            let y1 = e.pos.y.floor() as i32;
            for z in z0..=z1 {
                for x in x0..=x1 {
                    for y in y0..=y1 {
                        let pos = BlockPos::new(x, y, z);
                        let power_at_depth = pow - (e.pos.y - y as f64) as f32 * 0.5;
                        let below = BlockPos::new(x, y - 1, z);
                        let below_state = world.block_state(below);
                        // Vanilla `EntityRenderer.extractShadowPiece`: skip invisible render shapes first.
                        if shapes.render_shape_invisible(below_state) {
                            continue;
                        }
                        let brightness = max_local_raw_brightness(world, pos, sky_darken);
                        if brightness <= 3 {
                            continue;
                        }
                        // `isCollisionShapeFullBlock`: exactly one full 0..1 box.
                        let boxes = shapes.collision_boxes(below_state);
                        let full = boxes.len() == 1
                            && boxes[0].min == glam::DVec3::ZERO
                            && boxes[0].max == glam::DVec3::ONE;
                        if !full {
                            continue;
                        }
                        let b = lightmap_brightness(ambient_light, brightness);
                        let alpha = shadow_piece_alpha(power_at_depth, b);
                        if alpha <= 0.0 {
                            continue;
                        }
                        // The sRGB-target blend compensation (see `srgb_shadow_alpha`).
                        let alpha = srgb_shadow_alpha(alpha);
                        let r = 2.0 * radius as f64;
                        let bx = x as f64;
                        let bz = z as f64;
                        let by = y as f64;
                        let x01 = bx - e.pos.x;
                        let x11 = bx + 1.0 - e.pos.x;
                        let z01 = bz - e.pos.z;
                        let z11 = bz + 1.0 - e.pos.z;
                        let u0 = (-x01 / r) as f32 + 0.5;
                        let u1 = (-x11 / r) as f32 + 0.5;
                        let v0 = (-z01 / r) as f32 + 0.5;
                        let v1 = (-z11 / r) as f32 + 0.5;
                        let rel = |wx: f64, wy: f64, wz: f64| {
                            Vec3::new(
                                (wx - camera.x) as f32,
                                (wy - camera.y) as f32,
                                (wz - camera.z) as f32,
                            )
                        };
                        // `ShadowFeatureRenderer.prepare`: (x01,z01), (x01,z11), (x11,z11),
                        // (x11,z01), all at y01 = the block top.
                        let p = [
                            rel(bx, by, bz),
                            rel(bx, by, bz + 1.0),
                            rel(bx + 1.0, by, bz + 1.0),
                            rel(bx + 1.0, by, bz),
                        ];
                        let uv = [[u0, v0], [u0, v1], [u1, v1], [u1, v0]];
                        push_shadow_quad(&mut self.shadow_vertices, p, uv, alpha, rect);
                    }
                }
            }
        }
        if self.shadow_vertices.len() > self.shadow_capacity {
            self.shadow_capacity = self.shadow_vertices.len().next_power_of_two();
            self.shadow_buffer = make_vertex_buffer(&self.device, self.shadow_capacity);
        }
        if !self.shadow_vertices.is_empty() {
            self.queue.write_buffer(
                &self.shadow_buffer,
                0,
                bytemuck::cast_slice(&self.shadow_vertices),
            );
        }
    }

    /// Draws the batched entity shadows; the camera and lightmap bind groups must be set.
    pub fn draw_shadows<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.shadow_vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.shadow_pipeline);
        pass.set_bind_group(1, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.shadow_buffer.slice(..));
        pass.draw(0..self.shadow_vertices.len() as u32, 0..1);
        1
    }

    /// Number of shadow quads batched for the last frame (6 vertices each); tests/reporting.
    pub fn shadow_quad_count(&self) -> usize {
        self.shadow_vertices.len() / 6
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use haste_core::model::{ExperienceOrbDecoration, FallingBlockDecoration, PrimedTntDecoration};
    use haste_core::BlockStateId;

    fn instance(decoration: Option<Decoration>, invisible: bool) -> EntityInstance {
        EntityInstance {
            id: EntityId(1),
            model: haste_entity::models::box_for(1.0, 1.0),
            pos: glam::DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            scale: 1.0,
            pose: PoseOverrides::default(),
            body: BodyTransform::default(),
            body_scale: Vec3::ONE,
            skin: None,
            nametag: None,
            below_name: None,
            shadow_radius: 0.5,
            shadow_strength: 1.0,
            invisible,
            display_fire: false,
            width: 0.98,
            height: 0.98,
            is_discrete: false,
            name_tag_y: 1.0,
            hurt: 0.0,
            light: (15, 0),
            overlays: Default::default(),
            item: None,
            decoration,
            hands: [None, None],
            equipment: Vec::new(),
            head_item: None,
        }
    }

    #[test]
    fn the_disc_centre_darkens_like_vanillas_gamma_blend() {
        // The hovering sand of the offscreen scene: the camera 5.48 blocks away gives
        // `pow` 0.8826, half a block above the floor `powerAtDepth` 0.6326, full daylight.
        let b = lightmap_brightness(0.0, 15);
        let piece = shadow_piece_alpha(0.6326, b);
        assert!((piece - 0.3163).abs() < 1e-3, "piece alpha {piece}");
        // Vanilla's `ENTITY_SHADOW` blends that alpha on its plain UNORM target, so the
        // stored bytes darken by exactly it. Haste's sRGB target blends linear light, so
        // the vertex alpha is pre-distorted and the stored bytes come out the same.
        let vertex = srgb_shadow_alpha(piece);
        assert!((vertex - 0.5669).abs() < 1e-3, "vertex alpha {vertex}");
        let darkening = 1.0 - (1.0 - vertex).powf(1.0 / 2.2);
        assert!((darkening - 0.3163).abs() < 1e-3, "darkening {darkening}");
    }

    #[test]
    fn every_entity_is_shadow_eligible_except_the_hidden_ones() {
        // Every renderer's `shadowRadius` reaches the pass (boats 0.8, minecarts 0.7,
        // falling blocks and TNT 0.5, items and orbs 0.15; the radius-0 kinds draw
        // nothing through the `radius <= 0` gate, not an exclusion here).
        assert!(shadow_eligible(&instance(None, false)));
        assert!(shadow_eligible(&instance(
            Some(Decoration::FallingBlock(FallingBlockDecoration {
                block_state: BlockStateId::AIR,
                landed: false,
            })),
            false
        )));
        assert!(shadow_eligible(&instance(
            Some(Decoration::PrimedTnt(PrimedTntDecoration {
                block_state: BlockStateId::AIR,
                fuse: 80.0,
            })),
            false
        )));
        assert!(shadow_eligible(&instance(
            Some(Decoration::ExperienceOrb(ExperienceOrbDecoration {
                value: 1,
                age: 0.0,
            })),
            false
        )));
        // A landed falling block is hidden entirely (`shouldRender`), shadow included.
        assert!(!shadow_eligible(&instance(
            Some(Decoration::FallingBlock(FallingBlockDecoration {
                block_state: BlockStateId::AIR,
                landed: true,
            })),
            false
        )));
        // `Entity.isInvisible()`: no shadow (`extractShadow` skips it).
        assert!(!shadow_eligible(&instance(None, true)));
    }
}
