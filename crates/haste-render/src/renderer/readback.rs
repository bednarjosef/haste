// Screenshot readback (vanilla F2 `Screenshot.grab`): the async capture state
// machine on the windowed surface, the synchronous `read_back` of an offscreen
// renderer, and the copy-buffer unpacking both share.
use std::num::NonZeroU32;

use anyhow::{Context, Result};

use super::Renderer;

/// An in-flight screenshot readback: the copy is submitted and `map_async` posted, the
/// callback flips `status`, and `take_capture` collects it frames later without blocking.
pub(crate) struct Capture {
    buffer: std::sync::Arc<wgpu::Buffer>,
    status: CaptureFlag,
    width: u32,
    height: u32,
    /// Row stride of the padded copy buffer.
    stride: usize,
    /// Red/blue swapped on BGRA surfaces.
    bgra: bool,
}

/// Phases of an in-flight screenshot readback; the `map_async` callback advances
/// `Pending` to `Ready` or `Failed`, `take_capture` polls the flag.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CapturePhase {
    /// Copy submitted, map posted, callback not yet run.
    Pending,
    /// Mapped and readable.
    Ready,
    /// The map failed.
    Failed,
}

/// The phase shared with the `map_async` callback (a `CapturePhase` in an atomic).
#[derive(Clone, Debug, Default)]
pub(super) struct CaptureFlag(std::sync::Arc<std::sync::atomic::AtomicU8>);

impl CaptureFlag {
    pub(super) fn get(&self) -> CapturePhase {
        match self.0.load(std::sync::atomic::Ordering::Acquire) {
            1 => CapturePhase::Ready,
            2 => CapturePhase::Failed,
            _ => CapturePhase::Pending,
        }
    }
    pub(super) fn set(&self, phase: CapturePhase) {
        self.0
            .store(phase as u8, std::sync::atomic::Ordering::Release);
    }
}

impl Renderer {
    /// Read the last offscreen frame back as an RGBA image (screenshot tests).
    pub fn read_back(&mut self) -> Result<image::RgbaImage> {
        let off = self
            .offscreen
            .as_ref()
            .context("read_back requires an offscreen renderer")?;
        self.read_texture(off.color.as_image_copy(), off.width, off.height)
    }

    /// F2: capture the next rendered frame for `take_capture` (window renderers; the
    /// offscreen path has `read_back`). Ignored while a previous capture is still in
    /// flight: its buffer stays mapped until collected.
    pub fn request_capture(&mut self) {
        if self.capture.is_none() {
            self.capture_pending = true;
        }
    }

    /// Starts the readback of the just-rendered frame: submits the texture copy and
    /// posts `map_async` without blocking; the callback flips the shared status and
    /// `take_capture` collects the image a few frames later.
    pub(super) fn start_capture(&mut self, texture: wgpu::TexelCopyTextureInfo) {
        let (w, h) = self.size;
        let bytes_per_row = NonZeroU32::new((w * 4).next_multiple_of(256)).unwrap();
        let buffer = std::sync::Arc::new(self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot"),
            size: (bytes_per_row.get() * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        }));
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row.get()),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);
        let status = CaptureFlag::default();
        let flag = status.clone();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            flag.set(match r {
                Ok(()) => CapturePhase::Ready,
                Err(_) => CapturePhase::Failed,
            });
        });
        self.capture = Some(Capture {
            buffer,
            status,
            width: w,
            height: h,
            stride: bytes_per_row.get() as usize,
            bgra: matches!(
                self.surface_format,
                wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
            ),
        });
    }

    /// The frame captured after `request_capture`, once its async readback has landed
    /// (a few frames later); `None` while pending or with no capture in flight.
    pub fn take_capture(&mut self) -> Option<anyhow::Result<image::RgbaImage>> {
        let cap = self.capture.as_ref()?;
        // Non-blocking pump: runs the map callback once the copy has finished.
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        match cap.status.get() {
            CapturePhase::Pending => None,
            CapturePhase::Failed => {
                self.capture.take();
                Some(Err(anyhow::anyhow!(
                    "mapping the screenshot readback failed"
                )))
            }
            CapturePhase::Ready => {
                let cap = self.capture.take().expect("phase was Ready");
                let image = cap
                    .buffer
                    .slice(..)
                    .get_mapped_range()
                    .context("mapped screenshot range")
                    .map(|data| {
                        texel_rows_to_rgba(&data, cap.width, cap.height, cap.stride, cap.bgra)
                    })
                    .and_then(|pixels| {
                        image::RgbaImage::from_raw(cap.width, cap.height, pixels)
                            .context("screenshot size")
                    });
                cap.buffer.unmap();
                Some(image)
            }
        }
    }

    /// Copies a 4-byte-per-texel texture to a MAP_READ buffer, maps it and unwraps it
    /// into tightly packed RGBA rows.
    fn read_texture(
        &self,
        texture: wgpu::TexelCopyTextureInfo,
        w: u32,
        h: u32,
    ) -> Result<image::RgbaImage> {
        // Rows are padded to wgpu's 256-byte copy alignment.
        let bytes_per_row = NonZeroU32::new((w * 4).next_multiple_of(256)).unwrap();
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (bytes_per_row.get() * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        let bgra = matches!(
            self.surface_format,
            wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Bgra8Unorm
        );
        encoder.copy_texture_to_buffer(
            texture,
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row.get()),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit([encoder.finish()]);

        let (tx, rx) = std::sync::mpsc::channel();
        let buffer = std::sync::Arc::new(buffer);
        let map = buffer.clone();
        map.map_async(wgpu::MapMode::Read, .., move |r| {
            let _ = tx.send(r);
        });
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .ok();
        rx.recv()
            .context("waiting for the readback map")?
            .context("mapping the readback buffer")?;

        let data = map.get_mapped_range(..).context("mapped range")?.to_vec();
        map.unmap();

        let pixels = texel_rows_to_rgba(&data, w, h, bytes_per_row.get() as usize, bgra);
        Ok(image::RgbaImage::from_raw(w, h, pixels).expect("readback size"))
    }
}

/// Unpacks a GPU copy buffer into tightly packed RGBA rows: drops the 256-byte row
/// padding and, for BGRA surfaces, swaps the red and blue channels. Every pixel comes out
/// opaque, like vanilla `Screenshot.takeScreenshot`: the framebuffer's alpha is whatever the
/// last blend left (0.5 behind leaves and water), not a transparency the image should carry.
pub(super) fn texel_rows_to_rgba(
    data: &[u8],
    w: u32,
    h: u32,
    stride: usize,
    bgra: bool,
) -> Vec<u8> {
    let row = w as usize * 4;
    let mut pixels = Vec::with_capacity(row * h as usize);
    for y in 0..h as usize {
        let src = &data[y * stride..y * stride + row];
        if bgra {
            for px in src.as_chunks::<4>().0 {
                pixels.extend_from_slice(&[px[2], px[1], px[0], 255]);
            }
        } else {
            for px in src.as_chunks::<4>().0 {
                pixels.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
    }
    pixels
}
