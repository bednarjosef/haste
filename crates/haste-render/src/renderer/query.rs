// Read-only queries and secondary setters the client calls off the frame path
// (vanilla `Minecraft`'s accessors): stats, memory diagnostics, item registry
// and blur options.
use std::sync::Arc;

use haste_core::SectionPos;

use super::{Renderer, RendererCpuMemory};

impl Renderer {
    /// Text measurement over the default font (vanilla `Font.width` rules), for the HUD and
    /// tier-3 `Font` natives.
    pub fn text_metrics(&self) -> Arc<dyn haste_core::gui::TextMetrics> {
        self.gui.font().clone()
    }

    /// Item names by `minecraft:item` protocol id, so `GuiCommand::Item` can find models.
    /// The same call resolves the `minecraft:map_decoration_type` names the map renderer
    /// draws decorations with (the registry is not synced with data, so the static
    /// registry is the only source).
    pub fn set_item_registry(&mut self, registries: &haste_data::Registries) {
        let n = registries.len("minecraft:item");
        let names = (0..n as u32)
            .map(|i| {
                registries
                    .name("minecraft:item", haste_core::RegistryIndex(i))
                    .cloned()
                    .unwrap_or_else(|| haste_core::Identifier::minecraft("air"))
            })
            .collect::<Vec<_>>();
        self.entity.set_item_names(names.clone());
        self.gui.set_item_names(names);
        let n = registries.len("minecraft:map_decoration_type");
        let names = (0..n as u32)
            .filter_map(|i| {
                registries
                    .name(
                        "minecraft:map_decoration_type",
                        haste_core::RegistryIndex(i),
                    )
                    .cloned()
            })
            .collect::<Vec<_>>();
        self.entity.set_map_decoration_names(names);
    }

    /// The maps to draw (`MapTextureManager`'s data side): the client publishes the whole
    /// map set through an `Arc` swap whenever one changed, so this is cheap to call every
    /// frame and does nothing when the set did not move.
    pub fn set_maps(
        &mut self,
        maps: std::sync::Arc<std::collections::HashMap<i32, crate::entity::MapData>>,
    ) {
        self.entity.set_maps(maps);
    }

    /// Whether GUI blending happens on sRGB values like vanilla (false on backends without
    /// view formats, e.g. GL, where it blends in linear space).
    pub fn gui_blends_in_srgb(&self) -> bool {
        self.gpu.view_formats
    }

    /// CPU phase and GPU pass timings of the last frame.
    pub fn timings(&self) -> crate::timing::FrameTimings {
        self.timings
    }

    /// Target size in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The surface present mode in use.
    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.present_mode
    }

    /// The adapter this renderer runs on (name and backend, for bench reports).
    /// Terrain-arena memory diagnostics: arena count, the vertex+index bytes allocated
    /// (fixed-size GPU buffer pairs), the bytes actually used, plus the resident-section
    /// count.
    pub fn arena_memory(&self) -> (usize, u64, u64, usize) {
        let (arenas, bytes, used) = self.arenas.memory_stats();
        (arenas, bytes, used, self.sections.len())
    }

    /// Sections meshed but not yet uploaded (pending snapshots, queued results and
    /// in-flight jobs), plus the dirty sections held back by back-pressure: the feed
    /// settles only when everything the game thread marked has been meshed and uploaded.
    pub fn meshes_pending(&self) -> usize {
        self.mesher.pending_len() + self.mesh_carry.len()
    }

    /// The version of what the renderer wants meshed: bumped whenever a finished walk
    /// replaces the visible mask or the look crosses a 2-degree step (vanilla rebuilds
    /// `visibleSections` on those). The feed re-checks the dirty sections it held back
    /// as invisible whenever it changes.
    pub fn want_version(&self) -> u64 {
        self.want_version
    }

    /// Whether the visible set is final for `cam` / `view_distance`: no sightline data
    /// changed since the last walk, none is in flight, and the current mask was computed
    /// for exactly this camera and render distance. The feed's settle test waits on this,
    /// so screenshots and benches settle once meshing the visible sections can no longer
    /// reveal more of them.
    pub fn occlusion_settled(&self, cam: SectionPos, view_distance: i32) -> bool {
        !self.vis_dirty && !self.walk_in_flight && self.visible_for == Some((cam, view_distance))
    }

    /// CPU-side memory diagnostics for the bench's memory line: the block atlas's
    /// retained mip chain, the baked block models and the mesher's pending queue.
    pub fn cpu_memory(&self) -> RendererCpuMemory {
        RendererCpuMemory {
            atlas_bytes: self.atlas.pixel_bytes(),
            baked_bytes: self.baked.memory_bytes(),
            mesh_pending: self.mesher.pending_len() as u64,
        }
    }

    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.gpu.adapter.get_info()
    }

    /// Counters of the last GUI frame.
    pub fn gui_stats(&self) -> crate::gui::GuiStats {
        self.gui.stats
    }

    /// Particles posed in the last frame (debug: also logged once a second at debug level).
    pub fn particles_drawn(&self) -> u32 {
        self.particles.drawn
    }

    /// Entity instances that posed geometry in the last frame (models, decorations and
    /// dropped items; `FrameInput::entities` instances without geometry do not count).
    pub fn entities_drawn(&self) -> u32 {
        self.entity.drawn
    }

    /// The fog colour (sRGB) and distances of the last frame (`None` before the first render).
    pub fn last_fog(&self) -> Option<([f32; 3], crate::fog::FogData)> {
        self.last_fog
    }

    /// Vanilla `Options.menuBackgroundBlurriness`: the box-blur radius of an open screen's
    /// blurred background (0 = off, 10 = maximum; vanilla default 5).
    pub fn set_menu_blur_radius(&mut self, radius: u8) {
        self.menu_blur_radius = crate::blur::blur_radius(radius);
    }

    /// The title panorama for the next frame: its spin in degrees when the open screen
    /// extracted it (`GuiPipeline::panorama`), drawn under the GUI; `None` draws none. Holds
    /// for one `render` only, like vanilla's per-frame `PanoramaRenderState`.
    pub fn set_panorama(&mut self, spin: Option<f32>) {
        self.panorama.request(spin);
    }
}
