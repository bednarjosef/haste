//! The block entities posed into the entity vertex stream after prepare: one draw call
//! covers entities and block entities, each kind dispatched to its own emitter, plus the
//! beacon/gateway beam and end-portal passes with their own pipelines.

use super::*;
// The block entities' items come from their block entity data, whose components are not
// decoded: they draw without glint.
use haste_core::model::FoilType;

impl EntityRenderer {
    /// Poses the frame's block entities (chests, shulker boxes) into the same vertex stream as
    /// the entities, after `prepare`: one draw call covers both, and each texture lands in the
    /// shared atlas on first use (the per-vertex `rect` picks it). `visible` decides per
    /// instance whether it is drawn (frustum and section culling are the caller's, like the
    /// terrain's); returns how many were posed. `orientation` is the camera's orientation
    /// quaternion, for the conduit's billboarded eye.
    pub fn append_block_entities(
        &mut self,
        instances: &[crate::blockentity::BlockEntityInstance],
        camera: glam::DVec3,
        visible: impl Fn(&crate::blockentity::BlockEntityInstance) -> bool,
        partial: f32,
        assets: &mut Assets,
        orientation: glam::Quat,
    ) -> u32 {
        let start = self.vertices.len();
        let mut drawn = 0;
        self.beam_vertices.clear();
        self.glow_vertices.clear();
        self.portal_vertices.clear();
        for inst in instances {
            if !visible(inst) {
                continue;
            }
            // Beacon beams and the end gateways' beams go into the beam stream (their own
            // pipelines, no lightmap); end portals and gateway cubes into the portal stream
            // (position-only starfield shader). Both are emitted here and drawn by the
            // renderer's `draw_beams`/`draw_portals` after the entities. A gateway draws BOTH
            // (its cube and, while spawning or cooling down, its beam), so the beam branch
            // must not `continue` past the portal one.
            if matches!(
                inst.kind,
                crate::blockentity::BlockEntityKind::Beacon
                    | crate::blockentity::BlockEntityKind::EndGateway
            ) {
                let mut inner = std::mem::take(&mut self.beam_vertices);
                let mut glow = std::mem::take(&mut self.glow_vertices);
                let radius_scale = self.beam_radius_scale;
                let me = &mut *self;
                match inst.kind {
                    crate::blockentity::BlockEntityKind::Beacon => {
                        crate::blockentity::emit_beacon_beam(
                            &mut inner,
                            &mut glow,
                            inst,
                            camera,
                            radius_scale,
                            &mut |id| me.texture_rect(id, assets),
                        );
                    }
                    _ => crate::blockentity::emit_gateway_beam(
                        &mut inner,
                        &mut glow,
                        inst,
                        camera,
                        &mut |id| me.texture_rect(id, assets),
                    ),
                }
                self.beam_vertices = inner;
                self.glow_vertices = glow;
                drawn += 1;
                if matches!(inst.kind, crate::blockentity::BlockEntityKind::Beacon) {
                    continue;
                }
            }
            if matches!(
                inst.kind,
                crate::blockentity::BlockEntityKind::EndPortal
                    | crate::blockentity::BlockEntityKind::EndGateway
            ) {
                let mut out = std::mem::take(&mut self.portal_vertices);
                // Vanilla `TheEndPortalRenderer.TRANSFORMATION`: translate y +0.375, scale
                // (1, 0.375, 1); the gateway draws the whole cube.
                let m = (inst.pos.as_dvec3() - camera).as_vec3();
                let (root, layers) = match inst.kind {
                    crate::blockentity::BlockEntityKind::EndPortal => (
                        Mat4::from_translation(m)
                            * Mat4::from_translation(Vec3::new(0.0, 0.375, 0.0))
                            * Mat4::from_scale(Vec3::new(1.0, 0.375, 1.0)),
                        15.0,
                    ),
                    _ => (Mat4::from_translation(m), 16.0),
                };
                crate::blockentity::emit_portal_faces(&mut out, inst.portal_faces, layers, root);
                self.portal_vertices = out;
                drawn += 1;
                continue;
            }
            if let crate::blockentity::BlockEntityKind::Sign { hanging } = inst.kind {
                self.emit_sign_text(inst, hanging, camera);
                drawn += 1;
                continue;
            }
            if let crate::blockentity::BlockEntityKind::Banner { .. } = inst.kind {
                // The banner's angle is the same rule as the sign's (rotation segment or
                // wall facing) and lives in `sign_angle`.
                let mut out = std::mem::take(&mut self.vertices);
                let me = &mut *self;
                crate::blockentity::emit_banner(
                    &mut out,
                    inst,
                    camera,
                    inst.sign_angle,
                    &mut |texture| me.texture_rect(texture, assets),
                );
                self.vertices = out;
                drawn += 1;
                continue;
            }
            // The item and mob block entities draw through the item and cube emitters: the
            // item models come from the same asset chain the held and dropped items use
            // (cached per registry id, so a frame walks the cache, never the jar).
            if let Some(kind) = crate::blockentity::item_kind(&inst.kind) {
                let light = (inst.light.1, inst.light.0);
                let mut out = std::mem::take(&mut self.vertices);
                match kind {
                    crate::blockentity::ItemBlockEntity::Campfire(campfire) => {
                        for slot in 0..campfire.items.len() {
                            let Some(item) = campfire.get(slot) else {
                                continue;
                            };
                            let Some(model) = self.item_model_for(item, assets) else {
                                continue;
                            };
                            let m = crate::blockentity::campfire_item_matrix(
                                inst.pos,
                                inst.facing,
                                slot,
                                camera,
                            );
                            self.emit_item(
                                &mut out,
                                &model,
                                m,
                                light,
                                ItemDisplay::Fixed,
                                FoilType::None,
                                assets,
                            );
                        }
                    }
                    crate::blockentity::ItemBlockEntity::Shelf(shelf) => {
                        for slot in 0..shelf.items.len() {
                            let Some(item) = shelf.get(slot) else {
                                continue;
                            };
                            let Some(model) = self.item_model_for(item, assets) else {
                                continue;
                            };
                            let bbox = item_model_bbox(&model, &ItemDisplay::OnShelf);
                            let m = crate::blockentity::shelf_item_matrix(
                                inst.pos,
                                inst.facing,
                                slot,
                                shelf.align_to_bottom,
                                crate::blockentity::shelf_item_offset_y(
                                    bbox,
                                    shelf.align_to_bottom,
                                ),
                                camera,
                            );
                            self.emit_item(
                                &mut out,
                                &model,
                                m,
                                light,
                                ItemDisplay::OnShelf,
                                FoilType::None,
                                assets,
                            );
                        }
                    }
                    crate::blockentity::ItemBlockEntity::Vault(vault) => {
                        let copies = vault.copies();
                        if copies > 0 {
                            if let Some(model) = self.item_model_for(vault.item, assets) {
                                let bbox = item_model_bbox(&model, &ItemDisplay::Ground);
                                let base = crate::blockentity::vault_root(
                                    inst.pos,
                                    vault.spin_at(partial),
                                    camera,
                                );
                                let rand = |i: u32| seeded_float(vault.item.0, i);
                                self.emit_item_copies(
                                    &mut out,
                                    &model,
                                    base,
                                    copies,
                                    rand,
                                    light,
                                    ItemDisplay::Ground,
                                    bbox,
                                    FoilType::None,
                                    assets,
                                );
                            }
                        }
                    }
                    crate::blockentity::ItemBlockEntity::Brushable(b) => {
                        let Some(dir) = b.hit_direction else {
                            continue;
                        };
                        let Some(model) = self.item_model_for(b.item, assets) else {
                            continue;
                        };
                        let m = crate::blockentity::brushable_item_matrix(
                            inst.pos, dir, b.dusted, camera,
                        );
                        self.emit_item(
                            &mut out,
                            &model,
                            m,
                            light,
                            ItemDisplay::Fixed,
                            FoilType::None,
                            assets,
                        );
                    }
                }
                self.vertices = out;
                drawn += 1;
                continue;
            }
            if matches!(
                inst.kind,
                crate::blockentity::BlockEntityKind::Lectern
                    | crate::blockentity::BlockEntityKind::EnchantTable
                    | crate::blockentity::BlockEntityKind::Bell
                    | crate::blockentity::BlockEntityKind::DecoratedPot
                    | crate::blockentity::BlockEntityKind::Conduit
            ) {
                // The object-model block entities (books, bells, decorated pots, conduits):
                // one part list each, their own texture per part through the same atlas.
                let texture = match inst.kind {
                    crate::blockentity::BlockEntityKind::Lectern => {
                        crate::blockentity::book_texture()
                    }
                    crate::blockentity::BlockEntityKind::EnchantTable => {
                        crate::blockentity::book_texture()
                    }
                    crate::blockentity::BlockEntityKind::Bell => crate::blockentity::bell_texture(),
                    crate::blockentity::BlockEntityKind::DecoratedPot => {
                        crate::blockentity::pot_base_texture()
                    }
                    _ => crate::blockentity::conduit_shell_texture(),
                };
                let rect = self.texture_rect(&texture, assets);
                let mut out = std::mem::take(&mut self.vertices);
                let me = &mut *self;
                crate::blockentity::emit_model_block_entity(
                    &mut out,
                    inst,
                    camera,
                    orientation,
                    rect,
                    &mut |id| me.texture_rect(id, assets),
                );
                self.vertices = out;
                drawn += 1;
                continue;
            }
            // Moving pistons resolve their moved block's model through the same atlas (the
            // quads are cached per state string, so only the first frame pays the lookup).
            if matches!(inst.kind, crate::blockentity::BlockEntityKind::MovingPiston) {
                self.emit_piston(inst, camera, assets);
                drawn += 1;
                continue;
            }
            // A spawner's display entity is a cube model posed by the block entity's own
            // transform (`emit_spawner`), not by the item emitters.
            if let crate::blockentity::BlockEntityKind::Spawner(ref spawner) = inst.kind {
                if let Some(model) = spawner.model.as_ref() {
                    let rect = self.texture_rect(&model.texture, assets);
                    let mut out = std::mem::take(&mut self.vertices);
                    crate::blockentity::emit_spawner(&mut out, inst, model, rect, camera, partial);
                    self.vertices = out;
                    drawn += 1;
                }
                continue;
            }
            let rect = match inst.kind {
                crate::blockentity::BlockEntityKind::Chest(m) => {
                    self.texture_rect(crate::blockentity::chest_texture(m, inst.half), assets)
                }
                crate::blockentity::BlockEntityKind::Shulker(c) => {
                    self.texture_rect(crate::blockentity::shulker_texture(c), assets)
                }
                crate::blockentity::BlockEntityKind::Skull { kind, .. } => {
                    let t = crate::blockentity::skull_texture(kind);
                    self.texture_rect(&t, assets)
                }
                crate::blockentity::BlockEntityKind::Sign { .. } => unreachable!(),
                crate::blockentity::BlockEntityKind::Banner { .. } => unreachable!(),
                // The item and mob block entities left already.
                crate::blockentity::BlockEntityKind::Campfire(_)
                | crate::blockentity::BlockEntityKind::Spawner(_)
                | crate::blockentity::BlockEntityKind::Vault(_)
                | crate::blockentity::BlockEntityKind::Shelf(_)
                | crate::blockentity::BlockEntityKind::Brushable(_) => unreachable!(),
                crate::blockentity::BlockEntityKind::Lectern
                | crate::blockentity::BlockEntityKind::EnchantTable
                | crate::blockentity::BlockEntityKind::Bell
                | crate::blockentity::BlockEntityKind::DecoratedPot
                | crate::blockentity::BlockEntityKind::Conduit => unreachable!(),
                crate::blockentity::BlockEntityKind::Beacon
                | crate::blockentity::BlockEntityKind::EndPortal
                | crate::blockentity::BlockEntityKind::EndGateway
                | crate::blockentity::BlockEntityKind::MovingPiston => unreachable!(),
            };
            let out = &mut self.vertices;
            let emit = |out: &mut Vec<EntityVertex>,
                        model: &CubeModel,
                        part: &ModelPart,
                        pose: &PoseOverrides,
                        m: Mat4,
                        rect: [u32; 4],
                        light: (u8, u8)| {
                emit_part(out, model, part, pose, m, rect, light, 0.0);
            };
            crate::blockentity::emit_block_entity(out, inst, camera, rect, &emit);
            drawn += 1;
        }
        // One upload of the shared stream per frame: `prepare` already put the entities'
        // vertices up, so only the tail past `start` goes here — in full when the buffer
        // grew, because the fresh buffer holds no head (see `append_upload_range`).
        let range = append_upload_range(self.vertices.len(), self.vertex_capacity, start);
        if self.vertices.len() > self.vertex_capacity {
            self.vertex_capacity = self.vertices.len().next_power_of_two();
            self.vertex_buffer = make_vertex_buffer(&self.device, self.vertex_capacity);
        }
        if !range.is_empty() {
            self.queue.write_buffer(
                &self.vertex_buffer,
                (range.start * std::mem::size_of::<EntityVertex>()) as u64,
                bytemuck::cast_slice(&self.vertices[range]),
            );
        }
        if self.beam_vertices.len() > self.beam_capacity {
            self.beam_capacity = self.beam_vertices.len().next_power_of_two();
            self.beam_buffer = make_vertex_buffer(&self.device, self.beam_capacity);
        }
        if !self.beam_vertices.is_empty() {
            self.queue.write_buffer(
                &self.beam_buffer,
                0,
                bytemuck::cast_slice(&self.beam_vertices),
            );
        }
        if self.glow_vertices.len() > self.glow_capacity {
            self.glow_capacity = self.glow_vertices.len().next_power_of_two();
            self.glow_buffer = make_vertex_buffer(&self.device, self.glow_capacity);
        }
        if !self.glow_vertices.is_empty() {
            self.queue.write_buffer(
                &self.glow_buffer,
                0,
                bytemuck::cast_slice(&self.glow_vertices),
            );
        }
        if self.portal_vertices.len() > self.portal_capacity {
            self.portal_capacity = self.portal_vertices.len().next_power_of_two();
            self.portal_buffer = make_vertex_buffer(&self.device, self.portal_capacity);
        }
        if !self.portal_vertices.is_empty() {
            self.queue.write_buffer(
                &self.portal_buffer,
                0,
                bytemuck::cast_slice(&self.portal_vertices),
            );
        }
        self.drawn += drawn;
        drawn
    }

    /// Whether any beam or portal vertices were posed this frame (the draw calls are skipped).
    pub fn has_beams_or_portals(&self) -> bool {
        !self.beam_vertices.is_empty()
            || !self.glow_vertices.is_empty()
            || !self.portal_vertices.is_empty()
    }

    /// Draws the beacon/gateway beams: the opaque inner beams first (depth write, vanilla
    /// `RenderPipelines.BEACON_BEAM_OPAQUE`), then the translucent glow without depth write
    /// (`BEACON_BEAM_TRANSLUCENT`). Into the main pass; the camera bind group at group 0 and
    /// the entity atlas at group 1 must already be set.
    pub fn draw_beams<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        let mut calls = 0;
        if !self.beam_vertices.is_empty() {
            pass.set_pipeline(&self.beams.opaque);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.beam_buffer.slice(..));
            pass.draw(0..self.beam_vertices.len() as u32, 0..1);
            calls += 1;
        }
        if !self.glow_vertices.is_empty() {
            pass.set_pipeline(&self.beams.glow);
            pass.set_bind_group(1, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.glow_buffer.slice(..));
            pass.draw(0..self.glow_vertices.len() as u32, 0..1);
            calls += 1;
        }
        calls
    }

    /// Draws the end portal/gateway starfield cubes (vanilla `RenderPipelines.END_PORTAL` /
    /// `END_GATEWAY`): position-only vertices. The camera bind group must be set at group 0.
    pub fn draw_portals<'p>(&'p self, pass: &mut wgpu::RenderPass<'p>) -> u32 {
        if self.portal_vertices.is_empty() {
            return 0;
        }
        pass.set_pipeline(&self.beams.portal);
        pass.set_bind_group(1, &self.beams.portal_bind_group, &[]);
        pass.set_vertex_buffer(0, self.portal_buffer.slice(..));
        pass.draw(0..self.portal_vertices.len() as u32, 0..1);
        1
    }
}
