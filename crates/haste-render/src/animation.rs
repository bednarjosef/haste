// Animated atlas textures (water, lava, fire, portals): each game tick a
// worker thread prepares every animated sprite's changed pixels — the blend
// plus all mip levels, box-filtered exactly like the atlas chain (haste-assets
// `build_mip_chain` / `mean_linear`) — and issues the `write_texture` calls
// itself. The render thread never touches pixels or staging: `set_tick`
// hands the tick to the worker (deduped) and `update` only drains what the
// worker finished. When a tick is not ready yet, the previous frame stays.
//
// Thread safety: `wgpu::Queue` is `Send + Sync`, and a `write_texture` issued
// before a frame's `submit` (on any thread) is ordered before it, so writes
// the worker finished before the render thread's `update` are visible to that
// frame's passes; writes finishing later apply to the next frame.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use crossbeam_channel::{unbounded, Receiver, Sender};

/// Worker timings (for the perf test).
#[derive(Default)]
struct WorkerStats {
    prepare_nanos: AtomicU64,
    prepare_count: AtomicU64,
}

/// Animated sprite region uploads, split across two threads: a worker thread
/// (`haste-animation`) prepares each tick's changed pixels and issues the
/// atlas writes; the render thread only announces ticks (`set_tick`) and
/// collects what landed (`update`), never blending, downsampling, staging or
/// waiting.
pub(crate) struct SpriteAnimator {
    /// Last tick handed to the worker (dedupes per-frame calls).
    tick: Option<u64>,
    /// Ticks the worker finished writing; drained by `update`.
    done_rx: Receiver<u64>,
    /// Tick requests to the worker; dropping it ends the thread.
    tick_tx: Option<Sender<u64>>,
    /// Worker prepare+issue timings (for the perf test).
    #[cfg(test)]
    stats: Arc<WorkerStats>,
    worker: Option<JoinHandle<()>>,
}

impl SpriteAnimator {
    /// `atlas` is the renderer's built atlas, `texture`/`queue` the uploaded
    /// atlas texture and the shared queue the worker writes through.
    pub(crate) fn new(
        atlas: Arc<haste_assets::AtlasBuilder>,
        queue: Arc<wgpu::Queue>,
        texture: wgpu::Texture,
    ) -> anyhow::Result<Self> {
        let stats = Arc::new(WorkerStats::default());
        let (tick_tx, tick_rx) = unbounded();
        let (done_tx, done_rx) = unbounded::<u64>();
        let worker = std::thread::Builder::new()
            .name("haste-animation".into())
            .spawn({
                let stats = stats.clone();
                move || {
                    let sprites = atlas.animated_sprites();
                    // One scratch per mip level (level 0 included), reused
                    // across sprites and ticks: `write_texture` stages its
                    // data before returning, so the buffers are free to reuse
                    // once the call is done.
                    let mut scratch: Vec<Vec<u8>> =
                        vec![Vec::new(); atlas.mip_levels() as usize + 1];
                    // The tick whose pixels are on the GPU; `None` until the
                    // first prepared tick (the GPU starts on the sprites'
                    // initial frames, so the first tick rewrites everything).
                    let mut written: Option<u64> = None;
                    // Next tick to prepare, coalescing any already queued
                    // behind it (only the newest matters); `None` when the
                    // animator is dropped.
                    while let Ok(mut target) = tick_rx.recv() {
                        while let Ok(newer) = tick_rx.try_recv() {
                            target = newer;
                        }
                        let started = std::time::Instant::now();
                        for sprite in sprites {
                            // Same frame state: the shown pixels are unchanged.
                            if let Some(written) = written {
                                if haste_assets::frame_at(&sprite.schedule, written)
                                    == haste_assets::frame_at(&sprite.schedule, target)
                                {
                                    continue;
                                }
                            }
                            let rect = sprite.region.pixel_rect;
                            scratch[0].clear();
                            scratch[0].extend_from_slice(&sprite.frame_pixels(target));
                            let levels = 1 + downsample_region(&mut scratch, rect);
                            for level in 0..levels as u32 {
                                let [x, y, w, h] = rect;
                                write_region(
                                    &queue,
                                    &texture,
                                    level,
                                    [x >> level, y >> level, w >> level, h >> level],
                                    &scratch[level as usize],
                                );
                            }
                        }
                        written = Some(target);
                        stats
                            .prepare_nanos
                            .fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
                        stats.prepare_count.fetch_add(1, Ordering::Relaxed);
                        if done_tx.send(target).is_err() {
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            tick: None,
            done_rx,
            tick_tx: Some(tick_tx),
            #[cfg(test)]
            stats,
            worker: Some(worker),
        })
    }

    /// The worker's (prepare count, average prepare+issue cost in microseconds)
    /// (for tests).
    #[cfg(test)]
    fn worker_stats(&self) -> (u64, f64) {
        let prepares = self.stats.prepare_count.load(Ordering::Relaxed);
        let per_call = self.stats.prepare_nanos.load(Ordering::Relaxed) as f64
            / prepares.max(1) as f64
            / 1000.0;
        (prepares, per_call)
    }

    /// The game tick animated atlas textures (water, lava, fire, portals)
    /// should show. Hands the tick to the worker; duplicate ticks (per-frame
    /// calls between game ticks) wake nothing.
    pub(crate) fn set_tick(&mut self, tick: u64) {
        if self.tick == Some(tick) {
            return;
        }
        self.tick = Some(tick);
        if let Some(tx) = &self.tick_tx {
            let _ = tx.send(tick);
        }
    }

    /// Collects the newest tick the worker finished writing, if any. Cheap by
    /// construction — no pixel work, no staging, no waiting: a tick that is
    /// not ready yet simply keeps the previous frame. Returns the tick now on
    /// the GPU when the worker finished a new one since the last call.
    pub(crate) fn update(&mut self) -> Option<u64> {
        let mut latest = None;
        while let Ok(tick) = self.done_rx.try_recv() {
            latest = Some(tick);
        }
        latest
    }
}

impl Drop for SpriteAnimator {
    fn drop(&mut self) {
        // Drop the request channel first so the worker exits; join it.
        self.tick_tx.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
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

/// Downsamples `levels[0]` (the region's level-0 pixels) into `levels[1..]`
/// for as many levels as the region subdivides exactly (origin and size
/// divisible by the level's texel size — the packer's mip alignment holds for
/// atlas sprites) and as `levels` holds; returns the number of downsampled
/// levels written. The filter is the atlas chain's: an iterated plain 2x2 box
/// filter per level.
fn downsample_region(levels: &mut [Vec<u8>], rect: [u32; 4]) -> usize {
    let [x, y, w, h] = rect;
    let mut written = 0;
    for level in 1..levels.len() {
        let step = 1u32 << level;
        if x % step != 0 || y % step != 0 || w % step != 0 || h % step != 0 {
            break; // the region does not subdivide exactly at this level (nor deeper)
        }
        let (dw, dh) = (w >> level, h >> level);
        let (done, rest) = levels.split_at_mut(level);
        let dst = &mut rest[0];
        let src: &[u8] = &done[level - 1];
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
    use std::time::{Duration, Instant};

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
    /// the region back from the atlas texture, mip levels 0 and 1. The worker
    /// prepares and writes the tick off-thread, so this polls until it lands.
    /// Skips when no GPU adapter is available.
    #[test]
    fn gpu_updates_region_per_tick() {
        let Ok(gpu) = crate::pipeline::Gpu::new(None) else {
            eprintln!("skipped: no GPU adapter");
            return;
        };
        let atlas = Arc::new(synthetic_atlas());
        let (texture, _view, _sampler, _rects, _len) =
            crate::pipeline::upload_atlas(&gpu.device, &gpu.queue, &atlas).unwrap();
        let mut animator =
            SpriteAnimator::new(atlas.clone(), gpu.queue.clone(), texture.clone()).unwrap();
        animator.set_tick(2);
        let deadline = Instant::now() + Duration::from_secs(10);
        while animator.update() != Some(2) {
            assert!(Instant::now() < deadline, "worker never wrote tick 2");
            std::thread::sleep(Duration::from_millis(2));
        }
        let sprite = &atlas.animated_sprites()[0];
        let [x, y, w, h] = sprite.region.pixel_rect;
        let frame1 = sprite.frames[1].as_raw();
        assert_eq!(&read_region(&gpu, &texture, x, y, w, h, 0), frame1);
        let mut levels = vec![Vec::new(); atlas.mip_levels() as usize + 1];
        levels[0] = frame1.to_vec();
        let written = downsample_region(&mut levels, sprite.region.pixel_rect);
        assert!(written >= 1, "expected at least one downsampled level");
        assert_eq!(
            &read_region(&gpu, &texture, x / 2, y / 2, w / 2, h / 2, 1),
            &levels[1]
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

    /// `SpriteAnimator` on the real atlas (all animated sprites): the
    /// render-thread cost per tick (`update`, a channel drain) and the
    /// worker's cost of preparing and writing a tick. Needs the client jar
    /// and datagen reports in the cache (source scripts/env.sh) and a GPU.
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
        let mut built = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
        haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut built).unwrap();
        let atlas = Arc::new(built);
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
        let mut animator =
            SpriteAnimator::new(atlas.clone(), gpu.queue.clone(), texture.clone()).unwrap();

        // Render-thread cost: time every update() call, the draining ones
        // (a tick landed) and the empty ones alike.
        const RUNS: u64 = 100;
        let mut update_calls = 0u32;
        let mut update_total = Duration::ZERO;
        for tick in 1..=RUNS {
            animator.set_tick(tick);
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let started = Instant::now();
                let done = animator.update();
                update_total += started.elapsed();
                update_calls += 1;
                if done == Some(tick) {
                    break;
                }
                assert!(Instant::now() < deadline, "worker never wrote tick {tick}");
                std::thread::sleep(Duration::from_micros(100));
            }
        }
        // GPU side: how long the device takes to drain the queued writes.
        let gpu_started = Instant::now();
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let gpu_per_tick = gpu_started.elapsed() / RUNS as u32;
        // Same tick again: nothing changed, update must be a no-op.
        let started = Instant::now();
        for _ in 0..RUNS {
            animator.set_tick(RUNS);
            animator.update();
        }
        let unchanged = started.elapsed() / RUNS as u32;

        let (prepares, worker_per_call) = animator.worker_stats();
        eprintln!(
            "render thread, update() (channel drain, {update_calls} calls over {RUNS} ticks): {:.2} us/call",
            update_total.as_secs_f64() * 1e6 / update_calls as f64
        );
        eprintln!(
            "render thread, update with nothing new: {:.2} us",
            unchanged.as_secs_f64() * 1e6
        );
        eprintln!("worker, prepare + issue per tick ({prepares} ticks): {worker_per_call:.1} us");
        eprintln!(
            "device draining {RUNS} ticks of region writes: {:.1} us/tick",
            gpu_per_tick.as_secs_f64() * 1e6
        );
    }
}
