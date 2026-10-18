//! The entity shadow pass (`EntityRenderer.extractShadow`): round shadow discs on the full
//! block tops below entities, fading with distance, height and block brightness.

use super::*;

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
        shapes: &dyn CollisionShapes,
        sky_darken: i32,
        ambient_light: f32,
        entity_shadows: bool,
        assets: &mut Assets,
    ) {
        self.shadow_vertices.clear();
        if !entity_shadows {
            return;
        }
        let rect = self.texture_rect(&Identifier::minecraft("misc/shadow"), assets);
        for e in entities {
            if Some(e.id) == skip || e.invisible || e.decoration.is_some() {
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
                        // `isCollisionShapeFullBlock`: exactly one full 0..1 box.
                        let boxes = shapes.collision_boxes(below_state);
                        let full = boxes.len() == 1
                            && boxes[0].min == glam::DVec3::ZERO
                            && boxes[0].max == glam::DVec3::ONE;
                        if !full {
                            continue;
                        }
                        let brightness = max_local_raw_brightness(world, pos, sky_darken);
                        if brightness <= 3 {
                            continue;
                        }
                        let b = lightmap_brightness(ambient_light, brightness);
                        let alpha = shadow_piece_alpha(power_at_depth, b);
                        if alpha <= 0.0 {
                            continue;
                        }
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
