// The level render pass (vanilla `LevelRenderer.renderLevel`): everything drawn
// into the world's target — sky background, terrain layers, entities and block
// entities, weather and lightning, the hand's own pass, and the GUI on top.
use anyhow::{Context, Result};
use haste_core::{ChunkPos, SectionPos};

use super::{Camera, CameraUniform, FrameInput, FrameStats, Renderer, SectionDraw};

impl Renderer {
    /// Render one frame. Returns stats (frame time measured around the submit).
    pub fn render(&mut self, frame: &FrameInput) -> Result<FrameStats> {
        let started = std::time::Instant::now();
        let mut mark = started;
        let lap = |m: &mut std::time::Instant| {
            let now = std::time::Instant::now();
            let d = (now - *m).as_secs_f32() * 1000.0;
            *m = now;
            d
        };
        let aspect = self.size.0 as f32 / self.size.1 as f32;
        let view_proj = Self::view_projection(&frame.camera, aspect, frame.spin);
        // Entity culling needs the frustum before posing (vanilla's
        // `EntityRenderer.shouldRender`): the camera-relative view-projection's planes.
        let entity_frustum = Self::frustum_planes(&Self::view_projection(
            &Camera {
                pos: glam::DVec3::ZERO,
                ..frame.camera
            },
            aspect,
            frame.spin,
        ));
        let orientation =
            crate::blockentity::camera_orientation(frame.camera.yaw, frame.camera.pitch);
        self.entity.prepare(
            frame.entities,
            frame.camera_entity,
            frame.camera.pos,
            &entity_frustum,
            orientation,
            &mut self.assets,
        );
        {
            // The hand is drawn in view space: vanilla's `renderItemInHand` multiplies the
            // view rotation and its inverse into the pose, so they cancel and the hand's own
            // pose stack starts with `bobHurt` and `bobView` — the same bob the world's
            // projection carries, so hand and world wobble together. The plain camera roll
            // cancels with the view rotation and must not appear here.
            self.entity.prepare_hand(
                self.first_person.as_ref(),
                Self::bob_transform(&frame.camera),
                (frame.camera.yaw, frame.camera.pitch),
                frame.partial_tick,
                &mut self.assets,
            );
        }
        self.overlay.prepare(
            frame.camera.pos,
            frame.block_target.map(|(p, _)| p),
            frame.breaking,
            &self.baked,
        );
        self.nametags.clear();
        let gui_scale = frame.gui.scale.max(1) as f32;
        for tag in &self.entity.nametags {
            if let Some(t) =
                crate::gui::project_nametag(&view_proj, &frame.camera, tag, self.size, gui_scale)
            {
                self.nametags.push(t);
            }
        }
        let rot_view_proj = Self::view_projection(
            &Camera {
                pos: glam::DVec3::ZERO,
                ..frame.camera
            },
            aspect,
            frame.spin,
        );
        // The same matrix without the warp: the hand is drawn with the HUD projection in
        // vanilla (`GameRenderer.render3dHud` sets it before `renderItemInHand`), so it keeps
        // the plain projection while the world wobbles. The warp is a pure projection
        // multiplication, so an absent one leaves the matrix exactly as it was.
        let rot_view_proj_plain = if frame.spin.is_some() {
            Self::view_projection(
                &Camera {
                    pos: glam::DVec3::ZERO,
                    ..frame.camera
                },
                aspect,
                None,
            )
        } else {
            rot_view_proj
        };
        // Vanilla fog: `FogRenderer.setupFog`/`computeFogColor` pick the environment from the
        // camera's fluid, the player's effects and the dimension/biome attributes (all in
        // `frame.sky.fog`); the same path covers the overworld, the Nether and the End.
        let camera_forward = {
            let yaw = frame.camera.yaw.to_radians();
            let pitch = frame.camera.pitch.to_radians();
            [
                -pitch.cos() * yaw.sin(),
                -pitch.sin(),
                pitch.cos() * yaw.cos(),
            ]
        };
        let world_min_y = (self.min_section << 4) as f32;
        let (fog_srgb, fog_data) = crate::fog::frame_fog(
            &frame.sky,
            frame.partial_tick,
            camera_forward,
            frame.camera.pos.y as f32,
            world_min_y,
        );
        self.last_fog = Some((fog_srgb, fog_data));
        self.sky
            .prepare(&frame.sky, view_proj, rot_view_proj, &fog_data, fog_srgb);
        // The clouds (vanilla `CloudRenderer.prepare`, called from the level renderer's
        // translucent preparation with the frame's camera, game time and weather). The
        // mesh is camera-relative, so it takes the rotation-only projection.
        self.clouds.prepare(
            &frame.camera,
            &frame.sky,
            self.sky.kind,
            rot_view_proj,
            &fog_data,
            frame.partial_tick,
        );
        // The weather columns (vanilla draws them in the level pass after translucent
        // terrain, right where the sky overlays run) and the lightning bolts (the entities'
        // lightning decorations, camera-relative). Both submit camera-relative vertices,
        // so they take the rotation-only projection like the entity and particle passes.
        if let Some(weather) = frame.weather {
            self.weather.prepare(
                weather,
                &frame.camera,
                rot_view_proj,
                &fog_data,
                crate::sky::linear(fog_srgb),
                self.animation_tick as i64,
                frame.partial_tick,
            );
        }
        if frame.entities.iter().any(|e| {
            matches!(
                e.decoration,
                Some(haste_core::model::Decoration::Lightning(_))
            )
        }) {
            let camera = frame.camera.pos;
            self.lightning_bolts.clear();
            for e in frame.entities {
                if let Some(haste_core::model::Decoration::Lightning(bolt)) = &e.decoration {
                    self.lightning_bolts.push(crate::weather::LightningBolt {
                        pos: [
                            (e.pos.x - camera.x) as f32,
                            (e.pos.y - camera.y) as f32,
                            (e.pos.z - camera.z) as f32,
                        ],
                        seed: bolt.seed,
                    });
                }
            }
            self.weather.prepare_lightning(&self.lightning_bolts);
        } else {
            self.weather.prepare_lightning(&[]);
        }
        let fog_color = crate::sky::linear(fog_srgb);
        // Vanilla `GlobalSettingsUniform`: `((gameTime % 24000) + worldPartialTicks) / 24000`.
        let game_time_uniform =
            (self.frame_game_time.rem_euclid(24_000) as f32 + frame.partial_tick) / 24_000.0;
        let mut camera_uniform = CameraUniform {
            cam_pos: [
                frame.camera.pos.x as f32,
                frame.camera.pos.y as f32,
                frame.camera.pos.z as f32,
                0.0,
            ],
            // Camera-relative: the shaders subtract cam_pos from world positions.
            view_proj: rot_view_proj.to_cols_array_2d(),
            fog_color: [fog_color[0], fog_color[1], fog_color[2], 0.0],
            fog_env_start: fog_data.environmental_start,
            fog_env_end: fog_data.environmental_end,
            fog_rd_start: fog_data.render_distance_start,
            fog_rd_end: fog_data.render_distance_end,
            fog_sky_end: fog_data.sky_end,
            fog_cloud_end: fog_data.cloud_end,
            daylight: self.sky.daylight(),
            line_width: 0.0,
            aspect,
            game_time: game_time_uniform,
            hw_srgb: f32::from(self.hw_srgb),
            _pad: 0.0,
        };
        // Vanilla's outline width (Window.getAppropriateLineWidth): scales with the
        // framebuffer so it covers the same fraction of the screen at any resolution.
        // Pre-divided by the target height: the line shader multiplies it into a NDC offset.
        camera_uniform.line_width =
            (self.size.0 as f32 / 1920.0 * 2.5).max(2.5) * 0.5 / self.size.1 as f32;
        self.gpu
            .queue
            .write_buffer(&self.camera_uniform, 0, bytemuck::bytes_of(&camera_uniform));

        // The hand's own camera uniform, written only when the hand is drawn: vanilla's
        // `render3dHud` gives the hand its own projection (`hudProjection`: the fixed-base
        // hand FOV, near 0.05) with no bob, because `renderItemInHand` puts the bob onto the
        // hand's pose stack instead (the `bob_transform` prefix `prepare_hand` receives) —
        // the same bob the world's projection carries, applied exactly once. The hand is
        // submitted in view space, so there is no view matrix. The fog fields stay the
        // world's: the hand is under a metre from the eye, so like vanilla's own hand it
        // never reaches the first fog band.
        if self.entity.has_hand() {
            let hand_proj = glam::Mat4::perspective_rh(
                frame.camera.hand_fov_deg.to_radians(),
                aspect,
                0.05,
                4096.0,
            );
            let hand_uniform = CameraUniform {
                cam_pos: [0.0; 4],
                view_proj: hand_proj.to_cols_array_2d(),
                fog_color: [fog_color[0], fog_color[1], fog_color[2], 0.0],
                fog_env_start: fog_data.environmental_start,
                fog_env_end: fog_data.environmental_end,
                fog_rd_start: fog_data.render_distance_start,
                fog_rd_end: fog_data.render_distance_end,
                fog_sky_end: fog_data.sky_end,
                fog_cloud_end: fog_data.cloud_end,
                daylight: self.sky.daylight(),
                line_width: 0.0,
                aspect,
                game_time: game_time_uniform,
                hw_srgb: f32::from(self.hw_srgb),
                _pad: 0.0,
            };
            self.gpu.queue.write_buffer(
                &self.hand_camera_uniform,
                0,
                bytemuck::bytes_of(&hand_uniform),
            );
        }

        // The light texture's inputs for this frame (LightmapRenderStateExtractor.extract):
        // the dimension's fixed colours with the timeline's sky factor (and the rain blend,
        // WeatherAttributes.RAIN moving SKY_LIGHT_FACTOR towards 0.24 by rain level) plus the
        // frame's flicker, brightness, night vision, darkness, boss darkening and lightning.
        let mut inputs = self.lightmap_base;
        if frame.sky.dimension_has_sky {
            let mut factor = crate::lightmap::overworld_sky_factor(frame.sky.time_of_day);
            let rain = frame.sky.rain.clamp(0.0, 1.0);
            factor += (0.24 - factor) * rain * 0.3125;
            // ClientLevel's time-based SKY_LIGHT_FACTOR layer: a lightning flash is full sky.
            if self.lightmap_frame.lightning_flash {
                factor = 1.0;
            }
            inputs.sky_darken = 1.0 - factor;
        }
        inputs.block_light_flicker = self.lightmap_frame.block_light_flicker;
        inputs.gamma = self.lightmap_frame.gamma;
        inputs.night_vision = self.lightmap_frame.night_vision;
        inputs.darkness = self.lightmap_frame.darkness;
        inputs.boss_overlay_darkening = self.lightmap_frame.boss_overlay_darkening;
        self.lightmap.set_inputs(inputs);
        self.lightmap.upload(&self.gpu.queue);

        // Which target are we drawing into?
        // The acquired SurfaceTexture must outlive the pass (it presents on drop
        // after submit); hold it alongside the views.
        self.timings.prepare_ms = lap(&mut mark);
        if let Some(t) = &mut self.timer {
            t.poll(&self.gpu.device);
            self.timings.gpu_ms = t.latest;
            self.timings.gpu_sub_ms = t.latest_sub;
        }
        // A pending capture (F2) copies this frame's image out for `take_capture`; with
        // COPY_SRC on the surface that is the presented frame itself, and only without it
        // does this frame render into a temporary offscreen target (and not present).
        let capture_frame = self.capture_pending && self.offscreen.is_none();
        let mut held_surface_texture: Option<wgpu::SurfaceTexture> = None;
        let mut capture_color: Option<wgpu::Texture> = None;
        // The one colour view every pass of the frame renders through: the non-sRGB view of
        // the target, so the stored bytes are the shaders' sRGB-encoded values and the
        // translucent blends act on them like vanilla's plain UNORM main target.
        let (target_view, depth_view) = if capture_frame && !self.surface_copy {
            let cap = Self::make_offscreen(
                &self.gpu.device,
                self.size,
                self.surface_format,
                Self::gui_format_for(&self.gpu, self.surface_format),
            )?;
            let views = (cap.target_view.clone(), cap.depth_view.clone());
            capture_color = Some(cap.color);
            views
        } else {
            match (&self.offscreen, &self.depth) {
                (Some(off), _) => (off.target_view.clone(), off.depth_view.clone()),
                (None, Some(d)) => {
                    let surface = self.surface.as_ref().context("no surface")?;
                    let frame_tex = match surface.get_current_texture() {
                        wgpu::CurrentSurfaceTexture::Success(t)
                        | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                        wgpu::CurrentSurfaceTexture::Timeout
                        | wgpu::CurrentSurfaceTexture::Occluded => {
                            anyhow::bail!("surface frame unavailable (timeout/occluded)")
                        }
                        wgpu::CurrentSurfaceTexture::Outdated
                        | wgpu::CurrentSurfaceTexture::Lost
                        | wgpu::CurrentSurfaceTexture::Validation => {
                            anyhow::bail!("surface lost/outdated; reconfigure needed")
                        }
                    };
                    held_surface_texture = Some(frame_tex);
                    let tex = held_surface_texture.as_ref().unwrap();
                    (
                        tex.texture.create_view(&wgpu::TextureViewDescriptor {
                            format: Some(Self::gui_format_for(&self.gpu, self.surface_format)),
                            ..Default::default()
                        }),
                        d.clone(),
                    )
                }
                _ => anyhow::bail!("no render target"),
            }
        };
        let present = held_surface_texture.is_some();
        let _ = present;

        self.timings.acquire_ms = lap(&mut mark);
        // Frustum cull in camera-relative space (f32 precision far from the origin), into the
        // reused draw list.
        let planes = Self::frustum_planes(&rot_view_proj);
        let cam = frame.camera.pos;
        self.update_mesh_gate(planes, &frame.camera, aspect);
        self.update_occlusion_walk(cam);
        // Emit draws in the occlusion walk's visit (BFS) order — vanilla draws
        // its sections in `SectionOcclusionGraph`'s node order
        // (`addSectionsInFrustum` appends as the graph is visited), so the
        // opaque layers need no sort; only translucent is sorted back to front
        // (vanilla reverses the list for that layer). Bonus sections (newly
        // uploaded next to visible ones) follow; they are a few per frame. The
        // per-layer buckets replace the old layer-major sort of the whole list.
        let mut buckets = std::mem::take(&mut self.layer_buckets);
        for b in buckets.iter_mut() {
            b.clear();
        }
        let min_section = self.min_section;
        let sections = &self.sections;
        let emit = |buckets: &mut [Vec<SectionDraw>; 4], pos: SectionPos| {
            let Some(s) = sections.get(pos) else {
                return;
            };
            let si = s.pos.y - min_section;
            if !(0..32).contains(&si) {
                return;
            }
            let lo = [
                (s.aabb.0[0] as f64 - cam.x) as f32,
                (s.aabb.0[1] as f64 - cam.y) as f32,
                (s.aabb.0[2] as f64 - cam.z) as f32,
            ];
            let hi = [
                (s.aabb.1[0] as f64 - cam.x) as f32,
                (s.aabb.1[1] as f64 - cam.y) as f32,
                (s.aabb.1[2] as f64 - cam.z) as f32,
            ];
            if !Self::aabb_visible(&planes, lo, hi) {
                return;
            }
            let d = glam::Vec3::new(
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            )
            .length_squared();
            for l in sections.layers(s) {
                // Skip a direction group facing away from the camera.
                if l.dir < 6 && l.back_facing(lo, hi) {
                    continue;
                }
                // Sort key: ascending distance for opaque (early depth),
                // negated for translucent so ascending = far first.
                let translucent = l.layer == haste_assets::RenderLayer::Translucent;
                let i = match l.layer {
                    haste_assets::RenderLayer::Solid => 0,
                    haste_assets::RenderLayer::Cutout => 1,
                    haste_assets::RenderLayer::CutoutMipped => 2,
                    haste_assets::RenderLayer::Translucent => 3,
                };
                buckets[i].push(SectionDraw {
                    layer: l.layer,
                    arena: s.arena,
                    key: if translucent { -d } else { d },
                    first_index: l.first_index,
                    index_count: l.index_count,
                    base_vertex: s.base_vertex,
                    origin: [s.pos.x << 4, s.pos.y << 4, s.pos.z << 4],
                });
            }
        };
        self.for_each_visible_section(|pos| emit(&mut buckets, pos));
        // Group the opaque buckets by arena: the draw loop issues one multi-draw
        // call per run sharing a layer and an arena, and the visit order
        // interleaves the arenas, so ungrouped the frame pays thousands of draw
        // dispatches (2232 measured at rd 32 over 16 arenas). Depth-tested
        // layers do not depend on order (terrain faces never coincide).
        for b in buckets.iter_mut().take(3) {
            b.sort_unstable_by_key(|d| d.arena);
        }
        // Translucent back to front: the key is the negated distance, so
        // ascending key order is far first.
        if buckets[3].len() > 1 {
            buckets[3].sort_unstable_by(|a, b| a.key.total_cmp(&b.key));
        }
        let mut draws = std::mem::take(&mut self.draw_list);
        draws.clear();
        for b in buckets.iter() {
            draws.extend_from_slice(b);
        }
        self.layer_buckets = buckets;
        let sections_total = self.sections.len() as u32;
        let drawn = draws.len() as u32;
        let indices_drawn: u32 = draws.iter().map(|d| d.index_count).sum();
        self.origins.clear();
        self.origins.extend(draws.iter().map(|d| d.origin));
        if self.origins.len() > self.origin_capacity {
            self.origin_capacity = self.origins.len().next_power_of_two();
            self.origin_buffer = Self::make_origin_buffer(&self.gpu.device, self.origin_capacity);
        }
        if !self.origins.is_empty() {
            self.gpu.queue.write_buffer(
                &self.origin_buffer,
                0,
                bytemuck::cast_slice(&self.origins),
            );
        }
        if self.gpu.multi_draw {
            self.indirect.clear();
            for (i, d) in draws.iter().enumerate() {
                self.indirect.extend_from_slice(&[
                    d.index_count,
                    1,
                    d.first_index,
                    d.base_vertex,
                    i as u32,
                ]);
            }
            if draws.len() > self.indirect_capacity {
                self.indirect_capacity = draws.len().next_power_of_two();
                self.indirect_buffer =
                    Self::make_indirect_buffer(&self.gpu.device, self.indirect_capacity);
            }
            if !self.indirect.is_empty() {
                self.gpu.queue.write_buffer(
                    &self.indirect_buffer,
                    0,
                    bytemuck::cast_slice(&self.indirect),
                );
            }
        }

        // Block entities (chests, shulker boxes) join the entity vertex stream, culled like the
        // sections: the frustum plus the visible mask (and the newly uploaded bonus sections)
        // decide; vanilla compiles block entities into their section's render lists the same way.
        if !frame.block_entities.is_empty() {
            let planes_ref = &planes;
            let sections = &self.sections;
            let min_section = self.min_section;
            let column = |c: ChunkPos| self.visible_mask.column(c);
            // The conduit's eye is billboarded to the camera (vanilla `ConduitRenderer` reads
            // `camera.orientation`), which `Camera.setRotation` builds from yaw and pitch.
            let orientation =
                crate::blockentity::camera_orientation(frame.camera.yaw, frame.camera.pitch);
            // The beams' radius scale: 1 within 96 blocks of the beacon (vanilla
            // `BeaconRenderer.extract`; scoping is always 1).
            let mut radius_scale = 1.0f32;
            for inst in frame.block_entities {
                if matches!(inst.kind, crate::blockentity::BlockEntityKind::Beacon) {
                    // Vanilla `extract`: the horizontal distance to the block's centre
                    // (`Vec3.atCenterOf` then `horizontalDistance`), camera position vs
                    // centre, both y-zeroed.
                    let centre = inst.pos.as_dvec3() + glam::DVec3::new(0.5, 0.0, 0.5);
                    let d = ((centre.x - cam.x).powi(2) + (centre.z - cam.z).powi(2)).sqrt();
                    radius_scale =
                        radius_scale.max(crate::blockentity::beam_radius_scale(d as f32, false));
                }
            }
            self.entity.set_beam_radius_scale(radius_scale);
            // Vanilla `BlockEntityRenderer.shouldRender`: the renderer's view distance (64
            // blocks, the piston's 68, the beacon's render distance) bounds the draw; the fog
            // band's end is the effective render distance in blocks.
            let view_distance_blocks = fog_data.render_distance_end as f64;
            self.block_entities_drawn = self.entity.append_block_entities(
                frame.block_entities,
                cam,
                |inst| {
                    if !crate::blockentity::within_view_distance(
                        &inst.kind,
                        inst.pos,
                        cam,
                        view_distance_blocks,
                    ) {
                        return false;
                    }
                    let pos = inst.pos;
                    let lo = [
                        pos.x as f32 - cam.x as f32,
                        pos.y as f32 - cam.y as f32,
                        pos.z as f32 - cam.z as f32,
                    ];
                    let hi = [lo[0] + 1.0, lo[1] + 1.0, lo[2] + 1.0];
                    if !Self::aabb_visible(planes_ref, lo, hi) {
                        return false;
                    }
                    let section = pos.section();
                    let si = section.y - min_section;
                    let c = ChunkPos::new(section.x, section.z);
                    column(c) & (1u32 << si) != 0 || sections.get(section).is_some_and(|s| s.bonus)
                },
                frame.partial_tick,
                &mut self.assets,
                orientation,
            );
        } else {
            self.block_entities_drawn = 0;
        }

        self.timings.cull_ms = lap(&mut mark);
        let mut ran = [false; 3];
        // Main-pass sub-pass brackets (timing.rs `SubpassBrackets`): one bracket per sub-pass
        // in MAIN_SUBPASSES order; solid and cutout_mipped share the solid pipeline but get
        // separate brackets, since their arena runs interleave in the layer-major draw order.
        let mut brackets = crate::timing::SubpassBrackets::default();
        let timer = self.timer.as_ref();
        let mut draw_calls = 0u32;
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        {
            ran[0] = true;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The clear is the fog colour's sRGB-encoded bytes: the view is
                        // non-sRGB, so no hardware conversion happens and the raw value is
                        // what lands in the target (an sRGB view would have encoded the
                        // linear colour itself).
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: fog_srgb[0] as f64,
                            g: fog_srgb[1] as f64,
                            b: fog_srgb[2] as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(0)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            brackets.begin(&mut pass, timer, crate::timing::SKY);
            draw_calls += self.sky.draw_background(&mut pass);
            brackets.end(&mut pass, timer, crate::timing::SKY);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_bind_group(1, &self.atlas_bind_group, &[]);
            pass.set_bind_group(2, &self.lightmap_bind_group, &[]);

            // Layered draw order (RenderLayer Ord: Solid < Cutout < CutoutMipped
            // < Translucent). Section origins come from the instance-rate origin buffer, one
            // instance per draw; buffers are bound once per arena.
            pass.set_vertex_buffer(1, self.origin_buffer.slice(..));
            let mut entities_drawn = false;
            let mut bound_arena = u32::MAX;
            let mut bound_layer = None;
            let mut i = 0;
            while i < draws.len() {
                let d = &draws[i];
                let instance = i;
                // Entities after the opaque and cutout terrain, before translucent.
                if !entities_drawn && d.layer == haste_assets::RenderLayer::Translucent {
                    brackets.begin_entities(&mut pass, timer);
                    // Vanilla draws the entity shadows (a translucent, depth-tested pass) with
                    // the entities, then the block entities' special passes: the end
                    // portal/gateway starfield and the beacon beams (inner opaque, glow
                    // translucent without depth write). The special passes use their own bind
                    // groups at index 1, so the terrain's must be restored after them (the
                    // loop below sets the pipeline but not the bind groups again).
                    draw_calls += self.entity.draw_shadows(&mut pass);
                    draw_calls += self.entity.draw(&mut pass);
                    pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                    pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
                    draw_calls += self.entity.draw_portals(&mut pass);
                    draw_calls += self.entity.draw_beams(&mut pass);
                    pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                    // Close the entities bracket right after the entity draws, so the
                    // translucent terrain below is not timed as "entities".
                    brackets.end(&mut pass, timer, crate::timing::ENTITIES);
                    entities_drawn = true;
                    bound_layer = None;
                    bound_arena = u32::MAX;
                }
                if bound_layer != Some(d.layer) {
                    // CutoutMipped (leaves, vines) shares the solid *pipeline* (same shader
                    // state, mips differ in the sampler) but must keep the alpha discard:
                    // its sprites have transparent texels by vanilla's per-sprite layer rule
                    // (NativeImage.computeTransparency -> ChunkSectionLayer.byTransparency).
                    let (pipeline, sub) = match d.layer {
                        haste_assets::RenderLayer::Solid => (&self.pipeline_solid, 1),
                        haste_assets::RenderLayer::Cutout => (&self.pipeline_cutout, 2),
                        haste_assets::RenderLayer::CutoutMipped => (&self.pipeline_cutout, 3),
                        haste_assets::RenderLayer::Translucent => (&self.pipeline_translucent, 4),
                    };
                    brackets.begin_terrain_layer(&mut pass, timer, sub);
                    pass.set_pipeline(pipeline);
                    bound_layer = Some(d.layer);
                }
                if bound_arena != d.arena {
                    let a = &self.arenas.arenas[d.arena as usize];
                    pass.set_vertex_buffer(0, a.vertex.slice(..));
                    pass.set_vertex_buffer(1, self.origin_buffer.slice(..));
                    pass.set_index_buffer(a.index.slice(..), wgpu::IndexFormat::Uint32);
                    bound_arena = d.arena;
                }
                if self.gpu.multi_draw {
                    // One call for the run of draws sharing this layer and arena.
                    let end = draws[i..]
                        .iter()
                        .position(|e| e.layer != d.layer || e.arena != d.arena)
                        .map_or(draws.len(), |n| i + n);
                    let stride = std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>();
                    pass.multi_draw_indexed_indirect(
                        &self.indirect_buffer,
                        (i * stride) as u64,
                        (end - i) as u32,
                    );
                    draw_calls += 1;
                    i = end;
                } else {
                    pass.draw_indexed(
                        d.first_index..d.first_index + d.index_count,
                        d.base_vertex as i32,
                        instance as u32..instance as u32 + 1,
                    );
                    draw_calls += 1;
                    i += 1;
                }
            }
            if !entities_drawn {
                brackets.begin_entities(&mut pass, timer);
                draw_calls += self.entity.draw_shadows(&mut pass);
                draw_calls += self.entity.draw(&mut pass);
                pass.set_bind_group(1, &self.atlas_bind_group, &[]);
                pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
                draw_calls += self.entity.draw_portals(&mut pass);
                draw_calls += self.entity.draw_beams(&mut pass);
            }
            // Translucent is the last layer, so its bracket never sees a layer change:
            // `begin_particles` closes whatever terrain bracket is still open.
            brackets.begin_particles(&mut pass, timer);
            draw_calls += self.particles.draw(&mut pass);
            brackets.begin_overlays(&mut pass, timer);
            draw_calls += self.overlay.draw(&mut pass);
            draw_calls += self.clouds.draw(&mut pass);
            draw_calls += self.weather.draw(&mut pass);
            draw_calls += self.weather.draw_lightning(&mut pass);
            brackets.end(&mut pass, timer, crate::timing::OVERLAYS);
        }

        // First-person hand: its own pass over the world with the depth cleared, so it never
        // sinks into walls.
        if self.entity.has_hand() {
            ran[1] = true;
            // Without the warp: the hand is drawn with the HUD projection in vanilla, which
            // the screen warp never touches.
            if rot_view_proj_plain != rot_view_proj {
                camera_uniform.view_proj = rot_view_proj_plain.to_cols_array_2d();
                self.gpu.queue.write_buffer(
                    &self.camera_uniform,
                    0,
                    bytemuck::bytes_of(&camera_uniform),
                );
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hand"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.as_ref().and_then(|t| t.writes(1)),
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.hand_camera_bind_group, &[]);
            pass.set_bind_group(2, &self.lightmap_bind_group, &[]);
            draw_calls += self.entity.draw_hand(&mut pass);
        }

        // GUI on top (see `gui_pass`): the same non-sRGB view of the target.
        if let Some(n) = self.encode_gui(
            &mut encoder,
            frame,
            &target_view,
            held_surface_texture.as_ref(),
            capture_color.as_ref(),
        ) {
            ran[2] = true;
            draw_calls += n;
        }

        // Animated atlas textures: the animation worker writes each tick's
        // changed regions itself; this only collects what landed. Writes the
        // worker finished before this frame's submit (below) are ordered
        // before it, so this frame already sees them.
        self.animation.update();

        // Submit, then present (wgpu 30: present lives on the queue).
        let resolved = match &mut self.timer {
            Some(t) => t.resolve(&mut encoder, ran, brackets.ran()),
            None => false,
        };
        self.timings.encode_ms = lap(&mut mark);
        self.gpu.queue.submit([encoder.finish()]);
        if let Some(t) = &mut self.timer {
            t.after_submit(resolved);
        }
        self.timings.submit_ms = lap(&mut mark);
        // Submit the screenshot copy before present, so the GPU orders it ahead of the
        // present; the async map lands in `take_capture` a few frames later.
        if capture_frame {
            let src = match (&held_surface_texture, &capture_color) {
                (Some(tex), _) => tex.texture.as_image_copy(),
                (None, Some(color)) => color.as_image_copy(),
                (None, None) => anyhow::bail!("capture frame without a render target"),
            };
            self.start_capture(src);
        }
        if let Some(tex) = held_surface_texture {
            self.gpu.queue.present(tex);
        }
        self.capture_pending = false;
        self.timings.present_ms = lap(&mut mark);

        self.draw_list = draws;
        Ok(self.frame_stats(started, drawn, sections_total, indices_drawn, draw_calls))
    }
}
