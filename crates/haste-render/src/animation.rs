// Animated atlas textures (water, lava, fire, portals): each game tick the
// renderer re-uploads every animated sprite's atlas region — level 0 plus the
// matching sub-rectangle of every further mip level, box-filtered exactly like
// the atlas chain (haste-assets `build_mip_chain`; the linear-light average is
// Pomme-derived and duplicated here because haste-assets keeps it private —
// the chain-matching test below pins the two together).

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
                dst.extend_from_slice(&mean_linear(quad));
            }
        }
        written = level;
    }
    written
}

/// sRGB-correct 2x2 average (haste-assets `mean_linear`, Pomme: 10-bit linear
/// tables; alpha is a plain mean so cutout coverage survives).
fn mean_linear(pixels: [[u8; 4]; 4]) -> [u8; 4] {
    let mut output = [0u8; 4];
    for channel in 0..3 {
        let linear_sum: u32 = pixels
            .iter()
            .map(|pixel| u32::from(srgb_to_linear_10(pixel[channel])))
            .sum();
        output[channel] = linear_10_to_srgb((linear_sum / 4) as u16);
    }
    output[3] = (pixels.iter().map(|pixel| u32::from(pixel[3])).sum::<u32>() / 4) as u8;
    output
}

fn srgb_to_linear_10(channel: u8) -> u16 {
    static TABLE: std::sync::OnceLock<[u16; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index as f32 / 255.0;
            let linear = if x >= 0.04045 {
                (((f64::from(x) + 0.055) / 1.055).powf(2.4)) as f32
            } else {
                x / 12.92
            };
            (linear * 1023.0).round() as u16
        })
    })[channel as usize]
}

fn linear_10_to_srgb(channel: u16) -> u8 {
    static TABLE: std::sync::OnceLock<[u8; 1024]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        std::array::from_fn(|index| {
            let x = index as f32 / 1023.0;
            let srgb = if x >= 0.0031308 {
                (1.055 * f64::from(x).powf(1.0 / 2.4) - 0.055) as f32
            } else {
                12.92 * x
            };
            (srgb * 255.0).round().clamp(0.0, 255.0) as u8
        })
    })[usize::from(channel)]
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

    /// The region downsampling must reproduce the atlas chain's texels for the
    /// same level-0 pixels, at every mip level.
    #[test]
    fn region_mips_match_the_atlas_chain() {
        let atlas = synthetic_atlas();
        let sprite = &atlas.animated_sprites()[0];
        assert_eq!(sprite.frames.len(), 2);
        let frame = sprite.initial_frame();
        let rect @ [x, y, w, h] = sprite.region.pixel_rect;
        assert_eq!([x, y, w, h], [16, 0, 16, 16]);
        let mut mips = vec![Vec::new(); atlas.mip_levels() as usize];
        let levels = downsample_region(&mut mips, frame.as_raw(), rect);
        assert_eq!(levels, atlas.mip_levels() as usize);
        for level in 0..=levels as u32 {
            let dim = atlas.size() >> level;
            let rdim = (w >> level) as usize;
            let offset: usize = (0..level)
                .map(|l| ((atlas.size() >> l) as usize).pow(2) * 4)
                .sum();
            let mut expected = Vec::with_capacity(rdim * rdim * 4);
            for row in 0..rdim {
                let start = offset + (row * dim as usize + (x >> level) as usize) * 4;
                expected.extend_from_slice(&atlas.pixels()[start..start + rdim * 4]);
            }
            let actual: &[u8] = if level == 0 {
                frame.as_raw()
            } else {
                &mips[(level - 1) as usize]
            };
            assert_eq!(actual, &expected[..], "mip level {level}");
        }
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
}
