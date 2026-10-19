//! Vanilla `ItemModelGenerator`'s extrusion of `builtin/generated` layers (moved out of
//! `item.rs` unchanged).

use crate::model::{BakedQuad, Direction};

/// Vanilla's `ItemModelGenerator` extrusion of one `builtin/generated` layer: MIN_Z 7.5/16 and
/// MAX_Z 8.5/16 around the sprite, the south (z = 8.5/16) face with the sprite's UVs, the
/// north (z = 7.5/16) face with them mirrored, and a 1/16-deep side quad for every opaque
/// sprite pixel whose neighbour in that direction is transparent, sampling that pixel (the UVs
/// are inset 0.1 texel inside it, as vanilla's are). `image` is the sprite; without it
/// (an unreadable texture) only the two faces are baked. Positions are in 0..1 item space with
/// y up and the front towards +z, UVs 0..1, and every quad carries `tint_index`.
pub(super) fn generated_layer_quads(
    image: Option<&image::RgbaImage>,
    tint_index: u32,
) -> Vec<BakedQuad> {
    // Vanilla `ItemModelGenerator.bakeExtrudedSprite`: from (0,0,7.5) to (16,16,8.5), the south
    // face with the sprite's UVs and the north face with them mirrored.
    let mut quads = vec![
        face_quad(
            Direction::South,
            [0.0, 0.0, 7.5],
            [16.0, 16.0, 8.5],
            [0.0, 0.0, 16.0, 16.0],
            tint_index,
        ),
        face_quad(
            Direction::North,
            [0.0, 0.0, 7.5],
            [16.0, 16.0, 8.5],
            [16.0, 0.0, 0.0, 16.0],
            tint_index,
        ),
    ];
    let Some(image) = image else {
        return quads;
    };
    let (w, h) = image.dimensions();
    let x_scale = 16.0 / w.max(1) as f32;
    let y_scale = 16.0 / h.max(1) as f32;
    // Out of the sprite counts as transparent (vanilla's `isTransparent` returns true there).
    let opaque = |x: i32, y: i32| {
        (0..w as i32).contains(&x)
            && (0..h as i32).contains(&y)
            && image.get_pixel(x as u32, y as u32).0[3] != 0
    };
    for py in 0..h as i32 {
        for px in 0..w as i32 {
            if !opaque(px, py) {
                continue;
            }
            let x0 = px as f32 * x_scale;
            let x1 = x0 + x_scale;
            let y_top = 16.0 - py as f32 * y_scale;
            let y_bottom = y_top - y_scale;
            // The pixel's window in 0..16 sprite units, inset by vanilla's 0.1 texel.
            let u0 = x0 + 0.1 * x_scale;
            let u1 = x1 - 0.1 * x_scale;
            let v0 = py as f32 * y_scale + 0.1 * y_scale;
            let v1 = v0 + y_scale - 0.2 * y_scale;
            if !opaque(px, py - 1) {
                quads.push(face_quad(
                    Direction::Up,
                    [x0, y_top, 7.5],
                    [x1, y_top, 8.5],
                    [u0, v0, u1, v1],
                    tint_index,
                ));
            }
            if !opaque(px, py + 1) {
                quads.push(face_quad(
                    Direction::Down,
                    [x0, y_bottom, 7.5],
                    [x1, y_bottom, 8.5],
                    [u0, v0, u1, v1],
                    tint_index,
                ));
            }
            if !opaque(px - 1, py) {
                quads.push(face_quad(
                    Direction::West,
                    [x0, y_bottom, 7.5],
                    [x0, y_top, 8.5],
                    [u0, v1, u1, v0],
                    tint_index,
                ));
            }
            if !opaque(px + 1, py) {
                quads.push(face_quad(
                    Direction::East,
                    [x1, y_bottom, 7.5],
                    [x1, y_top, 8.5],
                    [u0, v1, u1, v0],
                    tint_index,
                ));
            }
        }
    }
    quads
}

/// One quad of the extruded sprite: positions from `from`/`to` (0..16 sprite units) on `dir` in
/// vanilla's `FaceInfo` order, and the UV rectangle `uv` (0..16 sprite units) mapped as
/// vanilla's `CuboidFace.UVs` corner cycle is.
fn face_quad(
    dir: Direction,
    from: [f32; 3],
    to: [f32; 3],
    uv: [f32; 4],
    tint_index: u32,
) -> BakedQuad {
    let mut quad = BakedQuad::from_element(
        dir,
        from.map(|v| v / 16.0),
        to.map(|v| v / 16.0),
        Some(&uv),
        None,
    );
    quad.texture = String::new();
    quad.cullface = None;
    quad.shade_face = Some(dir);
    quad.tint_index = Some(tint_index);
    quad.direction = dir;
    quad
}
