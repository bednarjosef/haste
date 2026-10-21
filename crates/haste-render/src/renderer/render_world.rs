// The level render pass (vanilla `LevelRenderer.renderLevel`): everything drawn
// into the world's target — sky background, terrain layers, entities and block
// entities, weather and lightning, the hand's own pass, and the GUI on top.
use anyhow::{Context, Result};
use haste_core::ChunkPos;

use super::{Camera, CameraUniform, FrameInput, FrameStats, Renderer};

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
        // Display billboards face the camera (`Camera.yRot()` / `xRot()`).
        self.entity
            .set_camera_rotation(frame.camera.yaw, frame.camera.pitch);
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

        self.timings.acquire_ms = lap(&mut mark);
        // Frustum cull in camera-relative space (f32 precision far from the origin), into the
        // reused draw list.
        let planes = Self::frustum_planes(&rot_view_proj);
        let cam = frame.camera.pos;
        self.update_mesh_gate(planes, &frame.camera, aspect);
        self.update_occlusion_walk(cam);
        // Emit draws in the occlusion walk's visit (BFS) order — vanilla draws
        // its sections in `SectionOcclusionGraph`'s node order, so the opaque
        // layers need no sort; only translucent is sorted back to front. See
        // `draw_list.rs` (the cull step, split out by the file-size rule).
        let (sections_total, drawn, indices_drawn) = self.collect_draws(&planes, cam);
        let draws = std::mem::take(&mut self.draw_list);

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
        // GPU occlusion culling (`gpu_cull.rs`): the early selection runs before the main
        // pass, which then splits around the depth pyramid and the late test.
        let culled = self.gpu_cull.active();
        if culled {
            self.gpu_cull.encode_early(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                rot_view_proj,
                self.size,
                &depth_view,
                &self.indirect_buffer,
            );
        }
        {
            ran[0] = true;
            let mut pass = self.begin_main_pass(
                &mut encoder,
                &target_view,
                &depth_view,
                Some(fog_srgb),
                !culled,
            );
            brackets.begin(&mut pass, timer, crate::timing::SKY);
            draw_calls += self.sky.draw_background(&mut pass);
            brackets.end(&mut pass, timer, crate::timing::SKY);
            self.bind_world_groups(&mut pass);
            let mut phase = super::terrain_draw::TerrainPhase::All;
            if culled {
                let mut untimed = crate::timing::SubpassBrackets::default();
                use super::terrain_draw::TerrainPhase::{Early, Late};
                draw_calls += self.draw_terrain(&mut pass, &draws, &mut untimed, Early);
                drop(pass);
                self.gpu_cull.encode_late(&mut encoder);
                pass = self.begin_main_pass(&mut encoder, &target_view, &depth_view, None, true);
                self.bind_world_groups(&mut pass);
                phase = Late;
            }
            draw_calls += self.draw_terrain(&mut pass, &draws, &mut brackets, phase);
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

        self.gpu_cull.record_stats(&self.gpu.device, &mut encoder);

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
        let submission = self.gpu.queue.submit([encoder.finish()]);
        if !present {
            self.offscreen_frames
                .submitted(&self.gpu.device, submission);
        }
        self.gpu_cull.after_submit();
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
