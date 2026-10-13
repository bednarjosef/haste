// Animated atlas textures (water, lava, fire, portals): each game tick the
// renderer re-uploads every animated sprite's atlas region — level 0 plus the
// matching sub-rectangle of every further mip level, box-filtered exactly like
// the atlas chain (haste-assets `build_mip_chain` / `mean_linear`).

/// The frame state `haste_assets::frame_at` returns: (index, next, blend).
type FrameState = (u32, u32, f32);

/// Per-tick animated sprite region uploads into the atlas texture. The renderer
/// sets the game tick (`set_tick`) once per tick and calls `update` per frame;
/// sprites whose shown pixels did not change are not re-uploaded.
pub(crate) struct SpriteAnimator {
    /// The tick set by the client; `None` until the first `set_tick`.
    tick: Option<u64>,
    /// The tick whose frames were last uploaded.
    uploaded: Option<u64>,
    /// Last uploaded frame state per sprite (the shown pixels are a pure
    /// function of it, so an equal state means unchanged texels).
    last: Vec<Option<FrameState>>,
    /// Scratch pixels per sprite and mip level (index `level - 1`), reused
    /// across ticks once grown.
    mips: Vec<Vec<Vec<u8>>>,
}

impl SpriteAnimator {
    /// `sprite_count` and `mip_levels` come from the built atlas
    /// (`animated_sprites().len()`, `mip_levels()`).
    pub(crate) fn new(sprite_count: usize, mip_levels: u32) -> Self {
        Self {
            tick: None,
            uploaded: None,
            last: vec![None; sprite_count],
            mips: vec![vec![Vec::new(); mip_levels as usize]; sprite_count],
        }
    }

    /// The game tick animated sprites show.
    pub(crate) fn set_tick(&mut self, tick: u64) {
        self.tick = Some(tick);
    }

    /// Uploads every animated sprite whose shown pixels changed since the last
    /// upload (a no-op while the tick is unchanged). `texture` is the atlas
    /// texture `upload_atlas` returned.
    pub(crate) fn update(
        &mut self,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        atlas: &haste_assets::AtlasBuilder,
    ) {
        let Some(tick) = self.tick else { return };
        if self.uploaded == Some(tick) {
            return;
        }
        self.uploaded = Some(tick);
        for (i, sprite) in atlas.animated_sprites().iter().enumerate() {
            let state = haste_assets::frame_at(&sprite.schedule, tick);
            if self.last[i] == Some(state) {
                continue; // same frame and blend: the region's texels are unchanged
            }
            self.last[i] = Some(state);
            let cow = sprite.frame_pixels(tick);
            let pixels: &[u8] = &cow;
            let rect = sprite.region.pixel_rect;
            write_region(queue, texture, 0, rect, pixels);
            let levels = downsample_region(&mut self.mips[i], pixels, rect);
            for (level, buffer) in self.mips[i][..levels].iter().enumerate() {
                let level = level as u32 + 1;
                let [x, y, w, h] = rect;
                write_region(
                    queue,
                    texture,
                    level,
                    [x >> level, y >> level, w >> level, h >> level],
                    buffer,
                );
            }
        }
    }
}

/// One `write_texture` of a tightly packed RGBA `rect` into `texture`.
fn write_region(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip_level: u32,
    rect: [u32; 4],
    pixels: &[u8],
) {
    let [x, y, w, h] = rect;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level,
            origin: wgpu::Origin3d { x, y, z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

/// Downsamples the region's level-0 `pixels` into `mips[level - 1]` for as many
/// levels as the region subdivides exactly (origin and size divisible by the
/// level's texel size — the packer's mip alignment holds for atlas sprites)
/// and as `mips` holds; returns the number of levels written. The filter is the
/// atlas chain's: an iterated plain 2x2 box filter per level.
fn downsample_region(mips: &mut [Vec<u8>], pixels: &[u8], rect: [u32; 4]) -> usize {
    let [x, y, w, h] = rect;
    let mut written = 0;
    for level in 1..=mips.len() {
        let step = 1u32 << level;
        if x % step != 0 || y % step != 0 || w % step != 0 || h % step != 0 {
            break; // the region does not subdivide exactly at this level (nor deeper)
        }
        let (dw, dh) = (w >> level, h >> level);
        let (done, rest) = mips.split_at_mut(level - 1);
        let dst = &mut rest[0];
        let src: &[u8] = if level == 1 { pixels } else { &done[level - 2] };
        let sw = w >> (level - 1);
        dst.clear();
        for py in 0..dh {
            for px in 0..dw {
                let mut quad = [[0u8; 4]; 4];
                for (i, (ox, oy)) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)]
                    .into_iter()
                    .enumerate()
                {
                    let s = (((py * 2 + oy) * sw + px * 2 + ox) * 4) as usize;
                    quad[i].copy_from_slice(&src[s..s + 4]);
                }
                dst.extend_from_slice(&haste_assets::mean_linear(quad));
            }
        }
        written = level;
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An atlas over a synthetic client jar holding one animated texture: a
    /// 16x32 vertical strip of two 16x16 frames, `frametime` 2, with mips.
    fn synthetic_atlas() -> haste_assets::AtlasBuilder {
        use std::io::Write;
        let dir =
            std::env::temp_dir().join(format!("haste-render-animation-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let jar = dir.join("client.jar");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&jar).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        // `Assets::open` checks these entries exist; content is irrelevant here.
        zip.start_file("assets/minecraft/blockstates/stone.json", options)
            .unwrap();
        zip.write_all(b"{\"variants\":{\"\":{\"model\":\"minecraft:block/stone\"}}}")
            .unwrap();
        zip.start_file("assets/minecraft/lang/en_us.json", options)
            .unwrap();
        zip.write_all(b"{}").unwrap();
        // The animated strip: two frames of non-uniform pixels (so the
        // linear-light averaging path is actually exercised).
        let mut strip = image::RgbaImage::new(16, 32);
        for y in 0..32u32 {
            for x in 0..16u32 {
                let frame = y / 16;
                strip.put_pixel(
                    x,
                    y,
                    image::Rgba([
                        (x * 15 + frame * 8) as u8,
                        (y * 7) as u8,
                        ((x ^ y) * 8 + frame * 3) as u8,
                        255,
                    ]),
                );
            }
        }
        let mut png = Vec::new();
        strip
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        zip.start_file("assets/minecraft/textures/block/anim.png", options)
            .unwrap();
        zip.write_all(&png).unwrap();
        zip.start_file("assets/minecraft/textures/block/anim.png.mcmeta", options)
            .unwrap();
        zip.write_all(b"{\"animation\":{\"frametime\":2}}").unwrap();
        zip.finish().unwrap();
        let mut assets = haste_assets::Assets::open(&jar, &dir.join("assets"), "test").unwrap();
        let id = haste_core::Identifier::parse("minecraft:block/anim").unwrap();
        haste_assets::AtlasBuilder::build(&mut assets, &[id], true).unwrap()
    }

    /// Uploads the 2-frame sprite at tick 2 (frametime 2: frame 1) and reads
    /// the region back from the atlas texture, mip levels 0 and 1. Skips when
    /// no GPU adapter is available.
    #[test]
    fn gpu_updates_region_per_tick() {
        let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let atlas = synthetic_atlas();
        let (texture, _view, _sampler, _rects, _len) =
            crate::pipeline::upload_atlas(&gpu.device, &gpu.queue, &atlas).unwrap();
        let mut animator = SpriteAnimator::new(atlas.animated_sprites().len(), atlas.mip_levels());
        animator.set_tick(2);
        animator.update(&gpu.queue, &texture, &atlas);
        let sprite = &atlas.animated_sprites()[0];
        let [x, y, w, h] = sprite.region.pixel_rect;
        let frame1 = sprite.frames[1].as_raw();
        assert_eq!(&read_region(&gpu, &texture, x, y, w, h, 0), frame1);
        let mut mips = vec![Vec::new(); atlas.mip_levels() as usize];
        let levels = downsample_region(&mut mips, frame1, sprite.region.pixel_rect);
        assert!(levels >= 1, "expected at least one downsampled level");
        assert_eq!(
            &read_region(&gpu, &texture, x / 2, y / 2, w / 2, h / 2, 1),
            &mips[0]
        );
    }

    /// Copies an RGBA rectangle out of one mip level of `texture` and maps it
    /// back (rows padded to wgpu's 256-byte copy alignment).
    fn read_region(
        gpu: &crate::pipeline::Gpu,
        texture: &wgpu::Texture,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
        mip_level: u32,
    ) -> Vec<u8> {
        let bytes_per_row = (w * 4).next_multiple_of(256);
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("animation-readback"),
            size: u64::from(bytes_per_row * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        let buffer = std::sync::Arc::new(buffer);
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        rx.recv().unwrap().unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        (0..h as usize)
            .flat_map(|row| {
                let start = row * bytes_per_row as usize;
                data[start..start + (w * 4) as usize].to_vec()
            })
            .collect()
    }

    /// `SpriteAnimator::update` on the real atlas (all animated sprites): 100
    /// calls over consecutive ticks (every sprite that changes that tick
    /// re-uploads) and 100 calls on an already-uploaded tick (nothing changes).
    /// Prints microseconds per call. Needs the client jar and datagen reports
    /// in the cache (source scripts/env.sh) and a GPU.
    #[test]
    #[ignore = "needs the client jar, datagen reports and a GPU (source scripts/env.sh)"]
    fn perf_animator_update_real_atlas() {
        let jar = haste_core::paths::client_jar();
        if !jar.exists() {
            eprintln!("ignored: needs the client jar in the cache");
            return;
        }
        let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
            eprintln!("ignored: no GPU adapter");
            return;
        };
        let mut assets = haste_assets::Assets::open(
            &jar,
            &haste_core::paths::cache_dir().join("assets"),
            "26.3",
        )
        .unwrap();
        let states = haste_assets::BlocksJsonStates::load(
            &haste_core::paths::datagen_reports().join("blocks.json"),
        )
        .unwrap();
        let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
        haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
        let sprites = atlas.animated_sprites();
        eprintln!(
            "atlas {}x{}, {} mip levels, {} animated sprites",
            atlas.size(),
            atlas.size(),
            atlas.mip_levels(),
            sprites.len()
        );
        let (texture, _view, _sampler, _rects, _len) =
            crate::pipeline::upload_atlas(&gpu.device, &gpu.queue, &atlas).unwrap();
        let mut animator = SpriteAnimator::new(sprites.len(), atlas.mip_levels());

        // Consecutive ticks: every sprite whose frame or blend moved re-uploads.
        const RUNS: u64 = 100;
        let mut changed_sprites = 0;
        let started = std::time::Instant::now();
        for tick in 1..=RUNS {
            animator.set_tick(tick);
            animator.update(&gpu.queue, &texture, &atlas);
            changed_sprites += sprites
                .iter()
                .filter(|s| {
                    let state = haste_assets::frame_at(&s.schedule, tick);
                    haste_assets::frame_at(&s.schedule, tick - 1) != state
                })
                .count();
        }
        gpu.queue.submit([]);
        // GPU side: how long the device takes to drain the queued writes.
        let gpu_started = std::time::Instant::now();
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let gpu_per_tick = gpu_started.elapsed() / RUNS as u32;
        let per_tick = started.elapsed() / RUNS as u32;
        // Same tick again: nothing changed, `update` must be a no-op.
        let started = std::time::Instant::now();
        for _ in 0..RUNS {
            animator.set_tick(RUNS);
            animator.update(&gpu.queue, &texture, &atlas);
        }
        let unchanged = started.elapsed() / RUNS as u32;

        let interpolating = sprites.iter().filter(|s| s.schedule.interpolate).count();
        eprintln!(
            "update, tick changed (avg {:.1} of {} sprites/tick over {RUNS} ticks): {:.1} us",
            changed_sprites as f64 / RUNS as f64,
            sprites.len(),
            per_tick.as_secs_f64() * 1e6
        );
        eprintln!(
            "update, tick unchanged (no-op): {:.2} us",
            unchanged.as_secs_f64() * 1e6
        );
        eprintln!(
            "device draining {RUNS} ticks of region writes: {:.1} us/tick",
            gpu_per_tick.as_secs_f64() * 1e6
        );
        eprintln!("{interpolating} sprites interpolate (blended pixels allocate per tick)");
    }
}
