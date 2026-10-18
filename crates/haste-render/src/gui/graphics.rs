//! Command list ingestion: each `GuiCommand` turned into quads (vanilla `GuiGraphics`: fills,
//! blits, nine-sliced and tiled sprites, scissors and blend switches).

use haste_assets::{Assets, SpriteMeta};
use haste_core::gui::{GuiBlend, GuiCommand, GuiFrame, TextStyle};
use haste_core::Identifier;

use super::*;

impl GuiRenderer {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn build(
        &mut self,
        frame: &GuiFrame,
        entities: &[haste_core::model::EntityInstance],
        nametags: &[ScreenNametag],
        target: (u32, u32),
        assets: &mut Assets,
        from: usize,
        to: usize,
        reset: bool,
    ) {
        if reset {
            self.vertices.clear();
            self.batches.clear();
        }
        // Name tags first, under the HUD, following `SubmitNodeCollection.submitNameTag`: a
        // black background box at `Options.textBackgroundOpacity` (0.25) and text at
        // `max((bg + 0.75) * 0.5, 0.5)` alpha. When not crouching (`see_through`) the opaque
        // solid pass is drawn on top; crouching keeps only the dimmer see-through pass, so a
        // sneaking player's tag is darker (`EntityRenderer.submitNameDisplay`).
        if reset {
            for tag in nametags {
                let start = self.vertices.len();
                let bg: [u8; 4] = [0, 0, 0, 64];
                let text_alpha = 0x80FF_FFFFu32;
                // The see-through line: background + semi-transparent text.
                self.nametag_line([0.0; 2], 0.0, &tag.text, text_alpha, bg, assets);
                if tag.see_through {
                    self.nametag_line([0.0; 2], 0.0, &tag.text, 0xFFFF_FFFF, [0, 0, 0, 0], assets);
                }
                // The scoreboard below-name line, one text line (`9 * 1.15`) under the name.
                if let Some(bn) = &tag.below_name {
                    let oy = 9.0 * 1.15;
                    self.nametag_line([0.0; 2], oy, bn, text_alpha, bg, assets);
                    if tag.see_through {
                        self.nametag_line([0.0; 2], oy, bn, 0xFFFF_FFFF, [0, 0, 0, 0], assets);
                    }
                }
                for v in &mut self.vertices[start..] {
                    v.pos = [tag.x + v.pos[0] * tag.scale, tag.y + v.pos[1] * tag.scale];
                }
            }
        }
        let scale = frame.scale.max(1) as f32;
        let mut translate: Vec<[f32; 2]> = Vec::new();
        let mut offset = [0.0f32; 2];
        let mut scissors: Vec<[f32; 4]> = Vec::new();
        let mut scissor: Option<[u32; 4]> = None;
        let mut blend = GuiBlend::Normal;
        let mut batch_start = self.vertices.len() as u32;
        for cmd in &frame.commands[from.min(frame.commands.len())..to.min(frame.commands.len())] {
            match cmd {
                GuiCommand::Fill {
                    x1,
                    y1,
                    x2,
                    y2,
                    color,
                } => {
                    let (x1, x2) = ((*x1).min(*x2), (*x1).max(*x2));
                    let (y1, y2) = ((*y1).min(*y2), (*y1).max(*y2));
                    self.solid(
                        offset,
                        [x1 as f32, y1 as f32, x2 as f32, y2 as f32],
                        rgba(*color),
                        rgba(*color),
                    );
                }
                GuiCommand::FillGradient {
                    x1,
                    y1,
                    x2,
                    y2,
                    top,
                    bottom,
                } => {
                    let (x1, x2) = ((*x1).min(*x2), (*x1).max(*x2));
                    let (y1, y2) = ((*y1).min(*y2), (*y1).max(*y2));
                    self.solid(
                        offset,
                        [x1 as f32, y1 as f32, x2 as f32, y2 as f32],
                        rgba(*top),
                        rgba(*bottom),
                    );
                }
                GuiCommand::Text {
                    x,
                    y,
                    text,
                    color,
                    shadow,
                } => self.text(
                    offset,
                    *x as f32,
                    *y as f32,
                    text,
                    *color,
                    *shadow,
                    TextStyle::default(),
                    assets,
                ),
                GuiCommand::StyledText {
                    x,
                    y,
                    text,
                    color,
                    shadow,
                    style,
                } => self.text(
                    offset, *x as f32, *y as f32, text, *color, *shadow, *style, assets,
                ),
                GuiCommand::Texture {
                    texture,
                    x,
                    y,
                    w,
                    h,
                    u,
                    v,
                    uw,
                    vh,
                    tex_w,
                    tex_h,
                    tint,
                    blend: cmd_blend,
                } => {
                    let cur = scissor;
                    self.switch(&mut scissor, &mut blend, cur, *cmd_blend, &mut batch_start);
                    let (rect, index) = self.lookup_ref(TexKeyRef::Texture(texture), assets);
                    let (iw, ih) = self.entries[index as usize].dims;
                    let kx = iw as f32 / (*tex_w).max(1) as f32;
                    let ky = ih as f32 / (*tex_h).max(1) as f32;
                    let u0 = rect[0] as f32 + u * kx;
                    let v0 = rect[1] as f32 + v * ky;
                    self.textured(
                        offset,
                        [*x as f32, *y as f32, (*x + *w) as f32, (*y + *h) as f32],
                        [u0, v0, u0 + uw * kx, v0 + vh * ky],
                        rect,
                        rgba(tint_or_white(*tint)),
                    );
                }
                GuiCommand::Sprite {
                    sprite,
                    x,
                    y,
                    w,
                    h,
                    tint,
                    blend: cmd_blend,
                } => {
                    let cur = scissor;
                    self.switch(&mut scissor, &mut blend, cur, *cmd_blend, &mut batch_start);
                    self.sprite(offset, sprite, *x, *y, *w, *h, *tint, assets)
                }
                GuiCommand::SpriteRegion {
                    sprite,
                    sprite_w,
                    sprite_h,
                    u,
                    v,
                    x,
                    y,
                    w,
                    h,
                    tint,
                    blend: cmd_blend,
                } => {
                    let cur = scissor;
                    self.switch(&mut scissor, &mut blend, cur, *cmd_blend, &mut batch_start);
                    let (rect, _) = self.lookup_ref(TexKeyRef::Sprite(sprite), assets);
                    let kx = rect[2] as f32 / (*sprite_w).max(1) as f32;
                    let ky = rect[3] as f32 / (*sprite_h).max(1) as f32;
                    let u0 = rect[0] as f32 + *u as f32 * kx;
                    let v0 = rect[1] as f32 + *v as f32 * ky;
                    self.textured(
                        offset,
                        [*x as f32, *y as f32, (*x + *w) as f32, (*y + *h) as f32],
                        [u0, v0, u0 + *w as f32 * kx, v0 + *h as f32 * ky],
                        rect,
                        rgba(tint_or_white(*tint)),
                    );
                }
                GuiCommand::Item { item, x, y, .. } => {
                    self.item(offset, item.0, *x as f32, *y as f32, assets)
                }
                GuiCommand::Entity {
                    entity,
                    x1,
                    y1,
                    x2,
                    y2,
                    scale: size,
                    mouse_x,
                    mouse_y,
                } => match entities.iter().find(|e| e.id == *entity) {
                    None => self.stats.unsupported += 1,
                    Some(e) => {
                        // Clipped to its box like vanilla's scissored entity widget.
                        let mut r = [
                            *x1 as f32 + offset[0],
                            *y1 as f32 + offset[1],
                            *x2 as f32 + offset[0],
                            *y2 as f32 + offset[1],
                        ];
                        if let Some(p) = scissors.last() {
                            r = [
                                r[0].max(p[0]),
                                r[1].max(p[1]),
                                r[2].min(p[2]),
                                r[3].min(p[3]),
                            ];
                        }
                        let outer = scissor;
                        let cur_blend = blend;
                        self.switch(
                            &mut scissor,
                            &mut blend,
                            Some(to_framebuffer(r, scale, target)),
                            cur_blend,
                            &mut batch_start,
                        );
                        self.entity_in_box(
                            offset,
                            e,
                            [*x1 as f32, *y1 as f32, *x2 as f32, *y2 as f32],
                            *size,
                            [*mouse_x, *mouse_y],
                            assets,
                        );
                        self.switch(&mut scissor, &mut blend, outer, cur_blend, &mut batch_start);
                    }
                },
                GuiCommand::ScissorPush { x1, y1, x2, y2 } => {
                    let mut r = [
                        *x1 as f32 + offset[0],
                        *y1 as f32 + offset[1],
                        *x2 as f32 + offset[0],
                        *y2 as f32 + offset[1],
                    ];
                    if let Some(p) = scissors.last() {
                        r = [
                            r[0].max(p[0]),
                            r[1].max(p[1]),
                            r[2].min(p[2]),
                            r[3].min(p[3]),
                        ];
                    }
                    scissors.push(r);
                    let next = Some(to_framebuffer(r, scale, target));
                    let cur = blend;
                    self.switch(&mut scissor, &mut blend, next, cur, &mut batch_start);
                }
                GuiCommand::ScissorPop => {
                    scissors.pop();
                    let next = scissors.last().map(|r| to_framebuffer(*r, scale, target));
                    let cur = blend;
                    self.switch(&mut scissor, &mut blend, next, cur, &mut batch_start);
                }
                GuiCommand::TranslatePush { dx, dy, .. } => {
                    translate.push(offset);
                    offset = [offset[0] + dx, offset[1] + dy];
                }
                GuiCommand::TranslatePop => {
                    offset = translate.pop().unwrap_or([0.0; 2]);
                }
            }
        }
        let end = self.vertices.len() as u32;
        if end > batch_start {
            self.batches.push(Batch {
                scissor,
                blend,
                start: batch_start,
                end,
            });
        }
    }

    /// Ends the current batch when the scissor or the blend mode is about to change, so each
    /// batch is one draw call with one pipeline (batching by blend, never per sprite).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn switch(
        &mut self,
        scissor: &mut Option<[u32; 4]>,
        blend: &mut GuiBlend,
        next_scissor: Option<[u32; 4]>,
        next_blend: GuiBlend,
        batch_start: &mut u32,
    ) {
        if *scissor == next_scissor && *blend == next_blend {
            return;
        }
        let end = self.vertices.len() as u32;
        if end > *batch_start {
            self.batches.push(Batch {
                scissor: *scissor,
                blend: *blend,
                start: *batch_start,
                end,
            });
        }
        *batch_start = end;
        *scissor = next_scissor;
        *blend = next_blend;
    }

    // ---- primitives ----

    pub(super) fn quad(
        &mut self,
        pos: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        color: [[u8; 4]; 4],
        rect: [u32; 4],
    ) {
        let rect = rect.map(|v| v as u16);
        for i in [0usize, 1, 2, 0, 2, 3] {
            self.vertices.push(GuiVertex {
                pos: pos[i],
                uv: uv[i],
                color: color[i],
                rect,
            });
        }
    }

    /// Axis-aligned solid rectangle (x1, y1, x2, y2) with top and bottom colours.
    pub(super) fn solid(&mut self, o: [f32; 2], r: [f32; 4], top: [u8; 4], bottom: [u8; 4]) {
        if r[0] >= r[2] || r[1] >= r[3] || (top[3] == 0 && bottom[3] == 0) {
            return;
        }
        let (x1, y1, x2, y2) = (r[0] + o[0], r[1] + o[1], r[2] + o[0], r[3] + o[1]);
        self.quad(
            [[x1, y1], [x1, y2], [x2, y2], [x2, y1]],
            [[0.0; 2]; 4],
            [top, bottom, bottom, top],
            [0; 4],
        );
    }

    /// Axis-aligned textured rectangle: `r` in GUI pixels, `uv` in atlas texels.
    pub(super) fn textured(
        &mut self,
        o: [f32; 2],
        r: [f32; 4],
        uv: [f32; 4],
        rect: [u32; 4],
        color: [u8; 4],
    ) {
        if r[0] >= r[2] || r[1] >= r[3] || color[3] == 0 || rect[2] == 0 || rect[3] == 0 {
            return;
        }
        let (x1, y1, x2, y2) = (r[0] + o[0], r[1] + o[1], r[2] + o[0], r[3] + o[1]);
        self.quad(
            [[x1, y1], [x1, y2], [x2, y2], [x2, y1]],
            [
                [uv[0], uv[1]],
                [uv[0], uv[3]],
                [uv[2], uv[3]],
                [uv[2], uv[1]],
            ],
            [color; 4],
            rect,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn sprite(
        &mut self,
        o: [f32; 2],
        sprite: &Identifier,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        tint: u32,
        assets: &mut Assets,
    ) {
        if w <= 0 || h <= 0 {
            return;
        }
        let (rect, index) = self.lookup_ref(TexKeyRef::Sprite(sprite), assets);
        let meta = self.entries[index as usize].meta.clone();
        let color = rgba(tint_or_white(tint));
        let (ax, ay, aw, ah) = (
            rect[0] as f32,
            rect[1] as f32,
            rect[2] as f32,
            rect[3] as f32,
        );
        let (fx, fy, fw, fh) = (x as f32, y as f32, w as f32, h as f32);
        match meta {
            SpriteMeta::Stretch => self.textured(
                o,
                [fx, fy, fx + fw, fy + fh],
                [ax, ay, ax + aw, ay + ah],
                rect,
                color,
            ),
            SpriteMeta::Tile { width, height } => {
                let kx = aw / width.max(1) as f32;
                let ky = ah / height.max(1) as f32;
                self.textured(
                    o,
                    [fx, fy, fx + fw, fy + fh],
                    [ax, ay, ax + fw * kx, ay + fh * ky],
                    rect,
                    color,
                )
            }
            SpriteMeta::NineSlice {
                width,
                height,
                border,
                stretch_inner,
            } => {
                let (sw, sh) = (width.max(1) as i32, height.max(1) as i32);
                if w == sw && h == sh {
                    self.textured(
                        o,
                        [fx, fy, fx + fw, fy + fh],
                        [ax, ay, ax + aw, ay + ah],
                        rect,
                        color,
                    );
                    return;
                }
                let kx = aw / sw as f32;
                let ky = ah / sh as f32;
                let [bl, bt, br, bb] = border.map(|b| b as i32);
                let (l, r) = (bl.min(w / 2), br.min(w / 2));
                let (t, b) = (bt.min(h / 2), bb.min(h / 2));
                // Destination and source (sprite units) spans of the three columns/rows.
                let cols = [
                    (0, l, 0, l),
                    (l, w - r, bl, sw - br),
                    (w - r, w, sw - r, sw),
                ];
                let rows = [
                    (0, t, 0, t),
                    (t, h - b, bt, sh - bb),
                    (h - b, h, sh - b, sh),
                ];
                for (ri, &(dy0, dy1, sy0, sy1)) in rows.iter().enumerate() {
                    for (ci, &(dx0, dx1, sx0, sx1)) in cols.iter().enumerate() {
                        if dx1 <= dx0 || dy1 <= dy0 || sx1 <= sx0 || sy1 <= sy0 {
                            continue;
                        }
                        let src = [
                            (ax + sx0 as f32 * kx) as u32,
                            (ay + sy0 as f32 * ky) as u32,
                            ((sx1 - sx0) as f32 * kx) as u32,
                            ((sy1 - sy0) as f32 * ky) as u32,
                        ];
                        let stretch = stretch_inner && ri == 1 && ci == 1;
                        let (span_u, span_v) = if stretch {
                            (src[2] as f32, src[3] as f32)
                        } else {
                            ((dx1 - dx0) as f32 * kx, (dy1 - dy0) as f32 * ky)
                        };
                        let (u0, v0) = (src[0] as f32, src[1] as f32);
                        self.textured(
                            o,
                            [
                                fx + dx0 as f32,
                                fy + dy0 as f32,
                                fx + dx1 as f32,
                                fy + dy1 as f32,
                            ],
                            [u0, v0, u0 + span_u, v0 + span_v],
                            src,
                            color,
                        );
                    }
                }
            }
        }
    }

    // ---- text ----
}
