// The visible sections as a dense list (vanilla `LevelRenderer.visibleSections`, the
// list `SectionOcclusionGraph` fills and the per-frame layer compilation walks): the
// occlusion walk's reached sections that are resident, in visit order, with the
// placement and layer ranges the draw list reads, copied out of the slab. The list is
// rebuilt only when the walk's mask or a slab record changed, so a frame whose
// sections did not change pays one linear pass over it instead of a hash lookup per
// walked position (about half of which miss: empty sections stay in the walk order).
// The list is kept grouped by arena, in visit order within each arena, so the draw
// list's per-layer buckets come out in the draw loop's run order with no per-frame sort.
// Each section's layer ranges are stored as `LayerGroups`: a render layer's facing groups
// in storage order with their offsets, so the frame turns the camera-facing groups into
// joined draws with a few bit operations instead of a test per group.

use haste_assets::RenderLayer;

use crate::mesher::{GROUPS, GROUP_ORDER};
use crate::quad_format::SpanKind;
use crate::section_slab::SlabLayer;

use super::Renderer;

/// One visible section: what the draw list needs without going back to the slab.
#[derive(Clone, Copy)]
pub(super) struct VisibleSection {
    pub slot: u32,
    pub arena: u32,
    pub base_vertex: u32,
    /// Block origin (section position times 16).
    pub origin: [i32; 3],
    /// World-space bounds.
    pub aabb: ([f32; 3], [f32; 3]),
    /// `VisibleList::groups` range of this section.
    groups_start: u32,
    groups_len: u32,
}

/// One render layer's facing groups of a section that lie back to back in the index
/// pool, by storage position (the index into `GROUP_ORDER`, the order the arena writes
/// them in): position `p` spans `start(p)..ends[p]`, empty when the group has no faces.
#[derive(Clone, Copy)]
pub(super) struct LayerGroups {
    pub layer: RenderLayer,
    pub kind: SpanKind,
    /// The draw list bucket (Solid, Cutout, CutoutMipped, Translucent).
    pub bucket: u8,
    /// Storage positions holding faces.
    present: u32,
    /// The highest position added (a later group must come after it to join).
    top: u8,
    first: u32,
    ends: [u32; GROUPS],
}

/// The storage position of each facing group (`GROUP_ORDER` inverted).
const POSITION: [u8; GROUPS] = {
    let mut p = [0u8; GROUPS];
    let mut i = 0;
    while i < GROUPS {
        p[GROUP_ORDER[i]] = i as u8;
        i += 1;
    }
    p
};

/// A section's camera-facing groups (`section_slab::front_groups`, bit per direction
/// group) as storage positions.
pub(super) fn front_positions(front: u16) -> u32 {
    let mut m = 0;
    for (p, &d) in GROUP_ORDER.iter().enumerate() {
        m |= ((u32::from(front) >> d) & 1) << p;
    }
    m
}

impl LayerGroups {
    fn new(l: &SlabLayer) -> Self {
        let mut g = LayerGroups {
            layer: l.layer,
            kind: l.kind,
            bucket: match l.layer {
                RenderLayer::Solid => 0,
                RenderLayer::Cutout => 1,
                RenderLayer::CutoutMipped => 2,
                RenderLayer::Translucent => 3,
            },
            present: 0,
            top: 0,
            first: l.first,
            ends: [l.first; GROUPS],
        };
        g.add(l);
        g
    }

    /// Whether `l` continues this run: same layer, indexed, right after the last group
    /// in the pool and later in storage order.
    fn continues(&self, l: &SlabLayer) -> bool {
        self.kind == SpanKind::Indexed
            && l.kind == SpanKind::Indexed
            && self.layer == l.layer
            && POSITION[l.dir as usize] > self.top
            && self.ends[self.top as usize] == l.first
    }

    fn add(&mut self, l: &SlabLayer) {
        let p = POSITION[l.dir as usize] as usize;
        self.present |= 1 << p;
        self.top = p as u8;
        for e in &mut self.ends[p..] {
            *e = l.first + l.count;
        }
    }

    fn start(&self, p: usize) -> u32 {
        if p == 0 {
            self.first
        } else {
            self.ends[p - 1]
        }
    }

    /// Calls `f(first, count)` for each maximal range of facing groups at the storage
    /// positions in `front` (`front_positions`): a range runs on through empty
    /// positions and stops at a group facing away.
    pub fn for_each_run(&self, front: u32, mut f: impl FnMut(u32, u32)) {
        let mut todo = front & self.present;
        let pass = front | !self.present;
        while todo != 0 {
            let p = todo.trailing_zeros();
            let end = (p + (pass >> p).trailing_ones()).min(GROUPS as u32);
            let first = self.start(p as usize);
            f(first, self.ends[end as usize - 1] - first);
            todo &= !((1u32 << end) - 1);
        }
    }
}

/// What the list was built from: the slab's change counter, the mask's generation, and
/// the two settings that change which sections the walk order yields.
type BuiltFrom = (u64, u64, bool, i32);

#[derive(Default)]
pub(super) struct VisibleList {
    sections: Vec<VisibleSection>,
    groups: Vec<LayerGroups>,
    built_from: Option<BuiltFrom>,
    /// Rebuild scratch: the sections in visit order, and the counting sort's offsets.
    visit_order: Vec<VisibleSection>,
    arena_offsets: Vec<u32>,
}

#[cfg(test)]
mod tests;

impl VisibleList {
    pub fn sections(&self) -> &[VisibleSection] {
        &self.sections
    }

    pub fn groups(&self, s: &VisibleSection) -> &[LayerGroups] {
        &self.groups[s.groups_start as usize..(s.groups_start + s.groups_len) as usize]
    }
}

impl Renderer {
    /// Brings `self.visible_list` up to date with the mask and the slab: rebuilt from
    /// `for_each_visible_section` when either changed since the last build.
    pub(super) fn refresh_visible_list(&mut self) {
        let from = (
            self.sections.generation(),
            self.visible_generation,
            self.cull_sections,
            self.min_section,
        );
        if self.visible_list.built_from == Some(from) {
            return;
        }
        let mut list = std::mem::take(&mut self.visible_list);
        list.visit_order.clear();
        list.groups.clear();
        let min_section = self.min_section;
        self.for_each_visible_section(|pos| {
            let Some((slot, s)) = self.sections.get_with_slot(pos) else {
                return;
            };
            if !(0..32).contains(&(s.pos.y - min_section)) {
                return;
            }
            let groups_start = list.groups.len();
            push_groups(&mut list.groups, self.sections.layers(s));
            list.visit_order.push(VisibleSection {
                slot,
                arena: s.arena,
                base_vertex: s.base_vertex,
                origin: [s.pos.x << 4, s.pos.y << 4, s.pos.z << 4],
                aabb: s.aabb,
                groups_start: groups_start as u32,
                groups_len: (list.groups.len() - groups_start) as u32,
            });
        });
        // Group by arena: the draw loop issues one multi-draw call per run sharing a
        // layer and an arena, and the visit order interleaves the arenas, so ungrouped
        // the frame pays thousands of draw dispatches (2232 measured at rd 32 over 16
        // arenas). Stable, so each arena keeps the walk's visit order.
        list.sections.clear();
        group_by_arena(
            &mut list.sections,
            &list.visit_order,
            |s| s.arena,
            &mut list.arena_offsets,
        );
        list.built_from = Some(from);
        self.visible_list = list;
    }
}

/// Appends a section's layer ranges to `out` as runs of back-to-back groups.
fn push_groups(out: &mut Vec<LayerGroups>, layers: &[SlabLayer]) {
    let start = out.len();
    for l in layers {
        if out.len() > start {
            let g = out.last_mut().unwrap();
            if g.continues(l) {
                g.add(l);
                continue;
            }
        }
        out.push(LayerGroups::new(l));
    }
}

/// Appends `src` to `out` grouped by `arena` (ascending), keeping the order within each
/// arena (a counting sort); `offsets` is reused scratch.
fn group_by_arena<T: Copy>(
    out: &mut Vec<T>,
    src: &[T],
    arena: impl Fn(&T) -> u32,
    offsets: &mut Vec<u32>,
) {
    let Some(&first) = src.first() else {
        return;
    };
    let arenas = src.iter().map(&arena).max().unwrap_or(0) as usize + 1;
    offsets.clear();
    offsets.resize(arenas + 1, 0);
    for d in src {
        offsets[arena(d) as usize + 1] += 1;
    }
    let base = out.len() as u32;
    let mut sum = base;
    for o in offsets.iter_mut() {
        sum += *o;
        *o = sum - *o;
    }
    out.resize(out.len() + src.len(), first);
    for d in src {
        let at = &mut offsets[arena(d) as usize + 1];
        out[*at as usize] = *d;
        *at += 1;
    }
}
