//! Decoding of one GUI frame from the bridge: `GuiBatch.frame`'s arrays (layout in
//! `java/haste/rt/GuiBatch.java`) into [`GuiCommand`]s.
use haste_core::gui::{CursorShape, GuiBlend, GuiCommand, TextStyle};
use haste_core::model::FoilType;
use haste_core::{EntityId, Identifier, RegistryIndex};
use jni::objects::{JIntArray, JObject, JObjectArray, JString};
use jni::Env;

use crate::error::{JvmError, Result};

// Opcodes; must match java/haste/rt/GuiBatch.java.
const FILL: i32 = 1;
const GRADIENT: i32 = 2;
const TEXT: i32 = 3;
const TEXTURE: i32 = 4;
const SPRITE: i32 = 5;
const ITEM: i32 = 6;
const SCISSOR_PUSH: i32 = 7;
const SCISSOR_POP: i32 = 8;
const TRANSLATE_PUSH: i32 = 9;
const TRANSLATE_POP: i32 = 10;
const SPRITE_REGION: i32 = 11;
const STYLED_TEXT: i32 = 12;
const ENTITY: i32 = 13;

/// The glint of an item record (`GfxImpl.foil` in Java: 0 none, 1 standard, 2 special;
/// vanilla `ItemStackRenderState.FoilType`'s order).
fn foil_of(code: i32) -> FoilType {
    match code {
        1 => FoilType::Standard,
        2 => FoilType::Special,
        _ => FoilType::None,
    }
}

/// The cursor shape for a code from `GuiBatch.frame` (`None` = the default arrow).
fn cursor_shape(code: i32) -> Option<CursorShape> {
    Some(match code {
        1 => CursorShape::Arrow,
        2 => CursorShape::IBeam,
        3 => CursorShape::Crosshair,
        4 => CursorShape::PointingHand,
        5 => CursorShape::ResizeNs,
        6 => CursorShape::ResizeEw,
        7 => CursorShape::ResizeAll,
        8 => CursorShape::NotAllowed,
        _ => return None,
    })
}

/// The blend mode of a blit (`GuiBatch`'s code, set by `GfxImpl.blend` in Java; 0 also falls back
/// to normal for codes this build does not know).
fn blend_of(code: i32) -> GuiBlend {
    match code {
        1 => GuiBlend::Multiply,
        2 => GuiBlend::Invert,
        3 => GuiBlend::Additive,
        _ => GuiBlend::Normal,
    }
}

/// One frame from `GuiBatch.frame`: its commands and the per-frame state beside them.
pub(super) struct DecodedFrame {
    pub commands: Vec<GuiCommand>,
    pub cursor: Option<CursorShape>,
    /// Command index the blur goes after, -1 for none (`GuiFrame::blur_after`).
    pub blur_after: i32,
    /// The title panorama's spin in degrees when a screen extracted it this frame (vanilla
    /// `GuiRenderState.panoramaRenderState`), `None` otherwise.
    pub panorama: Option<f32>,
}

pub(super) fn decode_frame(env: &mut Env, frame: JObject) -> Result<DecodedFrame> {
    // Safety: Bridge.render returns Object[]{int[], String[], int[4]}.
    let frame = unsafe { JObjectArray::<JObject>::from_raw(env, frame.into_raw()) };
    let ints = frame.get_element(env, 0)?;
    let ints = unsafe { JIntArray::from_raw(env, ints.into_raw()) };
    let n = ints.len(env)?;
    let mut buf = vec![0i32; n];
    ints.get_region(env, 0, &mut buf)?;
    let strs = frame.get_element(env, 1)?;
    let strs = unsafe { JObjectArray::<JString>::from_raw(env, strs.into_raw()) };
    let ns = strs.len(env)?;
    let mut strings = Vec::with_capacity(ns);
    for i in 0..ns {
        let s = strs.get_element(env, i)?;
        strings.push(s.try_to_string(env)?);
        env.delete_local_ref(s);
    }
    let cur = frame.get_element(env, 2)?;
    let cur = unsafe { JIntArray::from_raw(env, cur.into_raw()) };
    let mut code = [0i32; 4];
    cur.get_region(env, 0, &mut code)?;
    // The command index the blur goes after is a batch index; -1 means no blur this frame.
    let blur = if code[1] < 0 {
        None
    } else {
        Some(code[1] as usize)
    };
    let blur_after = match blur {
        Some(b) => decode(&buf[..b.min(buf.len())], &strings)?.len() as i32,
        None => -1,
    };
    Ok(DecodedFrame {
        commands: decode(&buf, &strings)?,
        cursor: cursor_shape(code[0]),
        blur_after,
        panorama: panorama_of(code[2], code[3]),
    })
}

/// The panorama state of `GuiBatch.frame`'s flag and spin bits: `None` unless a screen
/// extracted the panorama this frame.
fn panorama_of(flag: i32, spin_bits: i32) -> Option<f32> {
    (flag != 0).then(|| f32::from_bits(spin_bits as u32))
}

/// Decodes a frame's int records and string table into commands.
pub(crate) fn decode(b: &[i32], strings: &[String]) -> Result<Vec<GuiCommand>> {
    let bad = |what: &str| JvmError::Bridge(format!("malformed frame: {what}"));
    let s = |i: i32| {
        strings
            .get(i as usize)
            .cloned()
            .ok_or_else(|| bad("string index"))
    };
    let ident = |i: i32| -> Result<Identifier> {
        let t = strings.get(i as usize).ok_or_else(|| bad("string index"))?;
        Identifier::parse(t).map_err(|e| JvmError::Bridge(e.to_string()))
    };
    let f = f32::from_bits;
    let mut out = Vec::with_capacity(b.len() / 5);
    let mut i = 0;
    while i < b.len() {
        let need = |n: usize| {
            if i + n < b.len() + 1 {
                Ok(())
            } else {
                Err(bad("truncated record"))
            }
        };
        match b[i] {
            FILL => {
                need(6)?;
                out.push(GuiCommand::Fill {
                    x1: b[i + 1],
                    y1: b[i + 2],
                    x2: b[i + 3],
                    y2: b[i + 4],
                    color: b[i + 5] as u32,
                });
                i += 6;
            }
            GRADIENT => {
                need(7)?;
                out.push(GuiCommand::FillGradient {
                    x1: b[i + 1],
                    y1: b[i + 2],
                    x2: b[i + 3],
                    y2: b[i + 4],
                    top: b[i + 5] as u32,
                    bottom: b[i + 6] as u32,
                });
                i += 7;
            }
            TEXT => {
                need(6)?;
                out.push(GuiCommand::Text {
                    x: b[i + 1],
                    y: b[i + 2],
                    color: b[i + 3] as u32,
                    shadow: b[i + 4] != 0,
                    text: s(b[i + 5])?,
                });
                i += 6;
            }
            TEXTURE => {
                need(14)?;
                out.push(GuiCommand::Texture {
                    texture: ident(b[i + 1])?,
                    x: b[i + 2],
                    y: b[i + 3],
                    w: b[i + 4],
                    h: b[i + 5],
                    u: f(b[i + 6] as u32),
                    v: f(b[i + 7] as u32),
                    uw: f(b[i + 8] as u32),
                    vh: f(b[i + 9] as u32),
                    tex_w: b[i + 10] as u32,
                    tex_h: b[i + 11] as u32,
                    tint: b[i + 12] as u32,
                    blend: blend_of(b[i + 13]),
                });
                i += 14;
            }
            SPRITE => {
                need(8)?;
                out.push(GuiCommand::Sprite {
                    sprite: ident(b[i + 1])?,
                    x: b[i + 2],
                    y: b[i + 3],
                    w: b[i + 4],
                    h: b[i + 5],
                    tint: b[i + 6] as u32,
                    blend: blend_of(b[i + 7]),
                });
                i += 8;
            }
            ITEM => {
                need(6)?;
                out.push(GuiCommand::Item {
                    item: RegistryIndex(b[i + 1] as u32),
                    x: b[i + 2],
                    y: b[i + 3],
                    count: b[i + 4],
                    foil: foil_of(b[i + 5]),
                });
                i += 6;
            }
            SCISSOR_PUSH => {
                need(5)?;
                out.push(GuiCommand::ScissorPush {
                    x1: b[i + 1],
                    y1: b[i + 2],
                    x2: b[i + 3],
                    y2: b[i + 4],
                });
                i += 5;
            }
            SCISSOR_POP => {
                out.push(GuiCommand::ScissorPop);
                i += 1;
            }
            TRANSLATE_PUSH => {
                need(4)?;
                out.push(GuiCommand::TranslatePush {
                    dx: f(b[i + 1] as u32),
                    dy: f(b[i + 2] as u32),
                    dz: f(b[i + 3] as u32),
                });
                i += 4;
            }
            TRANSLATE_POP => {
                out.push(GuiCommand::TranslatePop);
                i += 1;
            }
            SPRITE_REGION => {
                need(12)?;
                out.push(GuiCommand::SpriteRegion {
                    sprite: ident(b[i + 1])?,
                    sprite_w: b[i + 2],
                    sprite_h: b[i + 3],
                    u: b[i + 4],
                    v: b[i + 5],
                    x: b[i + 6],
                    y: b[i + 7],
                    w: b[i + 8],
                    h: b[i + 9],
                    tint: b[i + 10] as u32,
                    blend: blend_of(b[i + 11]),
                });
                i += 12;
            }
            STYLED_TEXT => {
                need(7)?;
                let fl = b[i + 6];
                out.push(GuiCommand::StyledText {
                    x: b[i + 1],
                    y: b[i + 2],
                    color: b[i + 3] as u32,
                    shadow: b[i + 4] != 0,
                    text: s(b[i + 5])?,
                    style: TextStyle {
                        bold: fl & 1 != 0,
                        italic: fl & 2 != 0,
                        underline: fl & 4 != 0,
                        strikethrough: fl & 8 != 0,
                        obfuscated: fl & 16 != 0,
                    },
                });
                i += 7;
            }
            ENTITY => {
                need(9)?;
                out.push(GuiCommand::Entity {
                    entity: EntityId(b[i + 1]),
                    x1: b[i + 2],
                    y1: b[i + 3],
                    x2: b[i + 4],
                    y2: b[i + 5],
                    scale: f(b[i + 6] as u32),
                    mouse_x: f(b[i + 7] as u32),
                    mouse_y: f(b[i + 8] as u32),
                });
                i += 9;
            }
            op => return Err(bad(&format!("opcode {op}"))),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
