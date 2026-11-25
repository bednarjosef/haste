// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See docs/THIRD_PARTY.md.
//! Transparent-texel fills before mipping (vanilla `TextureUtil`).
/// Nearest-opaque-colour fill of transparent texels within `rect` (vanilla
/// `TextureUtil.solidify`: a BFS from every non-transparent texel; ties pick
/// whichever colour reached the texel first). Alpha is left untouched.
/// Public so the renderer can treat animated cutout sprites (fire) the same.
pub fn solidify(pixels: &mut [u8], size: u32, rect: [u32; 4]) {
    let [x, y, w, h] = rect;
    let index = |px: u32, py: u32| ((py * size + px) * 4) as usize;
    // Distance (in BFS steps) and colour of the nearest opaque texel.
    let mut distances = vec![u32::MAX; (w * h) as usize];
    let mut nearest = vec![[0u8; 3]; (w * h) as usize];
    let mut queue = std::collections::VecDeque::new();
    for py in 0..h {
        for px in 0..w {
            let at = (py * w + px) as usize;
            let alpha = pixels[index(x + px, y + py) + 3];
            if alpha != 0 {
                distances[at] = 0;
                nearest[at] = [
                    pixels[index(x + px, y + py)],
                    pixels[index(x + px, y + py) + 1],
                    pixels[index(x + px, y + py) + 2],
                ];
                queue.push_back((px, py));
            }
        }
    }
    while let Some((px, py)) = queue.pop_front() {
        let at = (py * w + px) as usize;
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (px as i32 + dx, py as i32 + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let (nx, ny) = (nx as u32, ny as u32);
            let nat = (ny * w + nx) as usize;
            if distances[nat] > distances[at] + 1 {
                distances[nat] = distances[at] + 1;
                nearest[nat] = nearest[at];
                queue.push_back((nx, ny));
            }
        }
    }
    for py in 0..h {
        for px in 0..w {
            let at = (py * w + px) as usize;
            if pixels[index(x + px, y + py) + 3] == 0 {
                let base = index(x + px, y + py);
                pixels[base..base + 3].copy_from_slice(&nearest[at]);
            }
        }
    }
}

/// Fill of transparent texels with the sprite's darkest colour, darkened to
/// 3/4 (vanilla `TextureUtil.fillEmptyAreasWithDarkColor`).
/// Public for the renderer's animated-sprite updates.
pub fn fill_empty_with_dark(pixels: &mut [u8], size: u32, rect: [u32; 4]) {
    let [x, y, w, h] = rect;
    let index = |px: u32, py: u32| ((py * size + px) * 4) as usize;
    let mut darkest = None;
    let mut min_brightness = u32::MAX;
    for py in 0..h {
        for px in 0..w {
            let at = index(x + px, y + py);
            if pixels[at + 3] != 0 {
                let brightness =
                    u32::from(pixels[at]) + u32::from(pixels[at + 1]) + u32::from(pixels[at + 2]);
                if brightness < min_brightness {
                    min_brightness = brightness;
                    darkest = Some([pixels[at], pixels[at + 1], pixels[at + 2]]);
                }
            }
        }
    }
    let Some(darkest) = darkest else { return };
    // vanilla TextureUtil: 3 * channel / 4 in int arithmetic (u8 would overflow)
    let dark = |c: u8| (3 * u16::from(c) / 4) as u8;
    let darkened = [dark(darkest[0]), dark(darkest[1]), dark(darkest[2])];
    for py in 0..h {
        for px in 0..w {
            let at = index(x + px, y + py);
            if pixels[at + 3] == 0 {
                pixels[at..at + 3].copy_from_slice(&darkened);
            }
        }
    }
}
