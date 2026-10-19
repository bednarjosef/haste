// Device and surface plumbing (vanilla `Window`/`Minecraft` window setup): the
// windowed and offscreen constructors, target formats, resize, and vsync.
use std::sync::Arc;

use anyhow::Result;

use crate::pipeline::Gpu;

use super::Renderer;

/// An owned display handle: the window Arc keeps the underlying connection
/// alive; the handle is fetched from the window on demand.
struct DisplayHandleWrapper {
    #[allow(dead_code)]
    window: Arc<winit::window::Window>,
}

impl core::fmt::Debug for DisplayHandleWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayHandleWrapper")
            .finish_non_exhaustive()
    }
}

impl winit::raw_window_handle::HasDisplayHandle for DisplayHandleWrapper {
    fn display_handle(
        &self,
    ) -> Result<winit::raw_window_handle::DisplayHandle<'_>, winit::raw_window_handle::HandleError>
    {
        self.window.display_handle()
    }
}

/// Offscreen render target (oracles screenshot tests): colour + depth textures the
/// renderer draws into when there is no window.
#[allow(dead_code)]
pub(crate) struct Offscreen {
    pub(crate) color: wgpu::Texture,
    pub(crate) color_view: wgpu::TextureView,
    /// Non-sRGB view of `color` for the GUI pass.
    pub(crate) gui_view: wgpu::TextureView,
    depth: wgpu::Texture,
    pub(crate) depth_view: wgpu::TextureView,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl Renderer {
    /// Windowed constructor (docs/INTERFACES.md).
    pub fn new(
        window: Arc<winit::window::Window>,
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        let display = DisplayHandleWrapper {
            window: window.clone(),
        };
        let display: Box<dyn crate::pipeline::ShareableDisplayHandle> = Box::new(display);
        let gpu = Gpu::new(Some(display))?;
        let surface = gpu
            .instance
            .create_surface(wgpu::SurfaceTarget::Window(Box::new(window.clone())))?;
        let cap = surface.get_capabilities(&gpu.adapter);
        let format = cap
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(cap.formats[0]);
        let size = window.inner_size();
        let mut renderer = Self::init(
            gpu,
            Some((surface, format)),
            (size.width.max(1), size.height.max(1)),
            assets,
            states,
        )?;
        renderer.configure_surface();
        Ok(renderer)
    }

    /// Offscreen constructor (oracles screenshot tests): renders into a texture.
    pub fn new_offscreen(
        w: u32,
        h: u32,
        assets: &mut haste_assets::Assets,
        states: &dyn haste_core::view::BlockStateTable,
    ) -> Result<Renderer> {
        let gpu = Gpu::new(None)?;
        Self::init(gpu, None, (w.max(1), h.max(1)), assets, states)
    }

    pub(super) fn make_origin_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-origins"),
            size: (capacity * 12) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub(super) fn make_indirect_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-indirect"),
            size: (capacity * std::mem::size_of::<wgpu::util::DrawIndexedIndirectArgs>()) as u64,
            usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The quad format's per-draw instance data (origin, record base, kind).
    pub(super) fn make_quad_instance_buffer(
        device: &wgpu::Device,
        capacity: usize,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("section-quad-instances"),
            size: (capacity * std::mem::size_of::<crate::quad_format::QuadInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// The format the GUI pass renders in: the non-sRGB view of the target where views are
    /// supported, else the target format itself.
    pub(super) fn gui_format_for(gpu: &Gpu, format: wgpu::TextureFormat) -> wgpu::TextureFormat {
        if gpu.view_formats {
            format.remove_srgb_suffix()
        } else {
            format
        }
    }

    pub(super) fn make_offscreen(
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        gui_format: wgpu::TextureFormat,
    ) -> Result<Offscreen> {
        let extra = [gui_format];
        let view_formats: &[wgpu::TextureFormat] = if gui_format != format { &extra } else { &[] };
        let (w, h) = size;
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-color"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats,
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen-depth"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Ok(Offscreen {
            gui_view: color.create_view(&wgpu::TextureViewDescriptor {
                format: Some(gui_format),
                ..Default::default()
            }),
            color_view: color.create_view(&Default::default()),
            depth_view: depth.create_view(&Default::default()),
            color,
            depth,
            width: w,
            height: h,
        })
    }

    pub(super) fn configure_surface(&mut self) {
        let Some(surface) = &self.surface else { return };
        let (w, h) = self.size;
        // COPY_SRC, when the surface supports it, lets F2 copy the presented frame out
        // directly; without it, the capture frame renders into an offscreen target.
        let copy_src = surface
            .get_capabilities(&self.gpu.adapter)
            .usages
            .contains(wgpu::TextureUsages::COPY_SRC);
        if !self.surface_copy_logged {
            self.surface_copy_logged = true;
            tracing::info!(
                "surface {} COPY_SRC: F2 captures {}",
                if copy_src { "supports" } else { "lacks" },
                if copy_src {
                    "the presented frame"
                } else {
                    "an offscreen frame"
                }
            );
        }
        self.surface_copy = copy_src;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | if copy_src {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            format: self.surface_format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: w,
            height: h,
            present_mode: self.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: {
                let gui = Self::gui_format_for(&self.gpu, self.surface_format);
                if gui != self.surface_format {
                    vec![gui]
                } else {
                    vec![]
                }
            },
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&self.gpu.device, &config);
        self.surface_state = Some((w, h, config));
        self.ensure_depth();
    }

    pub(super) fn ensure_depth(&mut self) {
        let depth = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width: self.size.0,
                height: self.size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24Plus,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some(depth.create_view(&Default::default()));
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.size = (w.max(1), h.max(1));
        if self.offscreen.is_some() {
            if let Ok(off) = Self::make_offscreen(
                &self.gpu.device,
                self.size,
                self.surface_format,
                Self::gui_format_for(&self.gpu, self.surface_format),
            ) {
                self.offscreen = Some(off);
            }
        } else {
            self.configure_surface();
        }
        self.blur.resize(
            &self.gpu.device,
            Self::gui_format_for(&self.gpu, self.surface_format),
            self.size,
        );
    }

    /// Vsync on (FIFO) or off (Mailbox, else Immediate, whichever the surface supports; FIFO
    /// when neither is). Reconfigures the surface; no effect offscreen.
    pub fn set_vsync(&mut self, on: bool) {
        let mode = match &self.surface {
            None => return,
            Some(surface) if !on => {
                let modes = surface.get_capabilities(&self.gpu.adapter).present_modes;
                [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
                    .into_iter()
                    .find(|m| modes.contains(m))
                    .unwrap_or(wgpu::PresentMode::Fifo)
            }
            Some(_) => wgpu::PresentMode::Fifo,
        };
        if mode != self.present_mode {
            self.present_mode = mode;
            self.configure_surface();
        }
    }
}
