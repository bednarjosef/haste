// Per-frame state the client feeds the renderer before `render` (vanilla's
// render-state extractors): the world sample, dimension and lightmap inputs,
// particle and entity preparation, and the per-tick setters.
use std::sync::Arc;

use haste_core::view::{BlockView, CollisionShapes};

use super::{Camera, FrameInput, LightmapFrame, Renderer, SpinWarp};

impl Renderer {
    /// Assembles the frame's `FrameStats` (the tail of `render`): the render pass's
    /// outputs, the mesh pipeline's uploads, and the mesh gate's pop-in count.
    pub(super) fn frame_stats(
        &self,
        started: std::time::Instant,
        drawn: u32,
        sections_total: u32,
        indices_drawn: u32,
        draw_calls: u32,
    ) -> super::FrameStats {
        super::FrameStats {
            frame_ms: started.elapsed().as_secs_f32() * 1000.0,
            chunk_sections_drawn: drawn,
            chunk_sections_total: sections_total,
            meshes_pending: self.meshes_pending() as u32,
            uploads: self.last_uploads.0,
            upload_bytes: self.last_uploads.1,
            upload_budget_ms: self.work_budget.last_budget_ms(),
            indices_drawn,
            draw_calls,
            block_entities_drawn: self.block_entities_drawn,
            visible_unmeshed: self.premesh.visible_unmeshed,
        }
    }

    /// The game tick animated atlas textures (water, lava, fire, portals) show;
    /// the client calls this once per game tick (20 Hz) and the next `render`
    /// re-uploads every changed sprite region.
    pub fn set_animation_tick(&mut self, tick: u64) {
        self.animation.set_tick(tick);
        self.animation_tick = tick;
        // The GUI's own animated textures (the fire and nether-portal overlays) share the tick.
        self.gui.set_animation_tick(tick);
    }

    /// The world's game time in ticks (vanilla `Level.getGameTime`), for the beacon beam's
    /// animation and the end portal's layer scroll; the next `render` folds it with the
    /// frame's partial tick into the camera uniform.
    pub fn set_game_time(&mut self, game_time: i64) {
        self.frame_game_time = game_time;
    }

    /// The first-person view: arm (empty hand) or held item; None draws nothing (third person,
    /// spectator). Stays until changed.
    pub fn set_first_person(&mut self, fp: Option<crate::entity::FirstPerson>) {
        self.first_person = fp;
    }

    /// The camera entity is a spectator this frame (`Camera.extractRenderState` reads it off
    /// the player). With the camera inside a solid-render block, [`Self::sample_world`] turns
    /// the section occlusion cull off for the frame.
    pub fn set_camera_spectator(&mut self, spectator: bool) {
        self.camera_spectator = spectator;
    }

    /// Samples what this frame's overlays need from the world (the renderer holds
    /// no world): the targeted block's outline boxes, the states of breaking
    /// blocks. Call before `render` with the frame you are about to render.
    pub fn sample_world(
        &mut self,
        world: &dyn BlockView,
        shapes: &haste_data::BlockShapes,
        frame: &FrameInput,
    ) {
        self.prepare_shadows(world, shapes, frame);
        // Vanilla `Camera.extractRenderState`: a spectator camera inside a solid-render
        // block draws with `smartCull` off, so the caves around it draw.
        let eye_block = haste_core::BlockPos::from_dvec3(frame.camera.pos);
        self.cull_sections = cull_sections(
            self.camera_spectator,
            self.baked.is_full_opaque(world.block_state(eye_block)),
        );
        let o = &mut self.overlay.world;
        o.target = frame.block_target.and_then(|(pos, _)| {
            let state = world.block_state(pos);
            let boxes = shapes.outline_boxes(state);
            (!boxes.is_empty()).then(|| (pos, boxes.to_vec()))
        });
        o.breaking.clear();
        for &(pos, stage) in frame.breaking {
            o.breaking.push((pos, world.block_state(pos), stage));
        }
    }

    /// The dimension being drawn: picks the sky (overworld, End, or fog colour only).
    pub fn set_dimension(&mut self, dim: &haste_core::game::DimensionInfo) {
        self.sky.kind = crate::sky::SkyKind::of(dim);
        // The dimension's fixed lightmap inputs (Nether/End ambient colours, no sky term);
        // `render` folds this frame's dynamic inputs into a copy each frame.
        self.lightmap_base = if dim.has_skylight {
            crate::lightmap::LightmapInputs::overworld_noon()
        } else if dim.name.path() == "the_end" {
            crate::lightmap::end()
        } else {
            crate::lightmap::nether()
        };
        self.min_section = dim.min_section();
        self.section_count = dim.section_count() as i32;
        self.section_vis.lock().clear();
        self.known_empty.lock().clear();
        self.visible_for = None;
        self.scheduled_for = None;
        self.walk_in_flight = false;
        self.clear_bonus();
    }

    /// This frame's dynamic lightmap inputs (flicker, brightness, night vision, darkness, boss
    /// overlay darkening, lightning flash), set by the client before `render`.
    pub fn set_lightmap_frame(&mut self, frame: LightmapFrame) {
        self.lightmap_frame = frame;
    }

    /// Poses this frame's particles from the game thread's per-tick snapshot (billboards,
    /// interpolated at `partial`, frustum-culled like vanilla's `QuadParticleGroup
    /// .extractRenderState`); no locks held — the snapshot is an `Arc` copy. Like
    /// [`Self::sample_world`]: call before `render` with the frame you are about to render.
    pub fn prepare_particles(
        &mut self,
        snapshot: &crate::particles::ParticleSnapshot,
        camera: Camera,
        partial: f32,
        spin: Option<SpinWarp>,
    ) {
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        // The camera-relative view-projection's planes, as the entity cull uses them.
        let rot_view_proj = Self::view_projection(
            &Camera {
                pos: glam::DVec3::ZERO,
                ..camera
            },
            aspect,
            spin,
        );
        let planes = Self::frustum_planes(&rot_view_proj);
        let yaw = camera.yaw.to_radians();
        let pitch = camera.pitch.to_radians();
        let forward = glam::Vec3::new(
            -pitch.cos() * yaw.sin(),
            -pitch.sin(),
            pitch.cos() * yaw.cos(),
        );
        self.particles
            .prepare(snapshot, camera, partial, &planes, forward);
    }

    /// Vanilla `Options.entityShadows`: draw the round entity shadows (default on).
    pub fn set_entity_shadows(&mut self, on: bool) {
        self.entity_shadows = on;
    }

    /// Vanilla `Entity.viewScale` factor for entity render distance (from
    /// `options.entityDistanceScaling`, 0.5..=5.0).
    pub fn set_view_scale(&mut self, scale: f32) {
        self.entity.set_view_scale(scale);
    }

    /// This frame's stuck arrows, stingers and shoulder parrots, sorted by entity id, with the
    /// stinger and parrot models the layers draw (`haste_entity::stuck_models`).
    pub fn set_stuck(
        &mut self,
        states: &[crate::entity::StuckInput],
        models: crate::entity::StuckModels,
    ) {
        self.entity.set_stuck(states, models);
    }

    /// Vanilla `Options.cloudStatus` (options.txt `renderClouds`): OFF draws no cloud pass,
    /// FAST the flat cells, FANCY the extruded boxes.
    pub fn set_cloud_status(&mut self, status: crate::clouds::CloudStatus) {
        self.clouds.set_status(status);
    }

    /// Vanilla `Options.cloudRange` (options.txt `cloudRange`, 2..=128 chunks, default
    /// 128): the radius of the cloud mesh around the camera.
    pub fn set_cloud_range(&mut self, range_chunks: u32) {
        self.clouds.set_range(range_chunks);
    }

    /// Vanilla `WorldBorderRenderState` for this frame: the border's square, its status tint
    /// and the render distance in blocks. `None` (no border known) draws no wall.
    pub fn set_world_border(&mut self, border: Option<crate::world_border::WorldBorderFrame>) {
        self.world_border.set_border(border);
    }

    /// Number of shadow quads batched for the last frame (tests/reporting).
    pub fn shadow_quad_count(&self) -> usize {
        self.entity.shadow_quad_count()
    }

    /// Extracts and batches the entity shadows for `frame` (vanilla
    /// `EntityRenderer.extractShadow`/`finalizeRenderState`). Called by [`Self::sample_world`];
    /// call it directly when you render without `sample_world` (tests) and want shadows.
    pub fn prepare_shadows(
        &mut self,
        world: &dyn BlockView,
        shapes: &haste_data::BlockShapes,
        frame: &FrameInput,
    ) {
        // `Level.getSkyDarken` = (int)(15 - SKY_LIGHT_LEVEL) off the day timeline; skyless
        // dimensions never use it (`getRawBrightness` reads only the block light there).
        let sky_darken = if frame.sky.dimension_has_sky {
            (15.0 * (1.0 - crate::lightmap::overworld_sky_factor(frame.sky.time_of_day))).floor()
                as i32
        } else {
            0
        };
        // `DimensionType.ambientLight`: 0.1 in the Nether (and skyless custom dimensions), 0
        // in the overworld and End.
        let ambient = if self.sky.kind == crate::sky::SkyKind::None {
            0.1
        } else {
            0.0
        };
        self.entity.prepare_shadows(
            frame.entities,
            frame.camera_entity,
            frame.camera.pos,
            world,
            shapes,
            sky_darken,
            ambient,
            self.entity_shadows,
            &mut self.assets,
        );
    }

    /// Uploads the particle atlas of `types` as the particle texture (call after
    /// `ParticleTypes::build`, once).
    pub fn set_particle_types(&mut self, types: &Arc<crate::particles::ParticleTypes>) {
        self.particles.set_types(types);
    }
}

/// Whether the section occlusion cull runs this frame. Vanilla `Camera.extractRenderState`
/// sets `smartCull = false` for a spectator camera inside a solid-render block (the player is
/// inside a hill: every section must draw or the screen goes empty).
pub(super) fn cull_sections(camera_spectator: bool, camera_in_solid: bool) -> bool {
    !(camera_spectator && camera_in_solid)
}
