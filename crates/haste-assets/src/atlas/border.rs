// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/renderer/chunk/atlas.rs, reused under GPL-3.0 and reworked
// for Haste: CPU-side RGBA buffers instead of Vulkan images (the wgpu renderer
// uploads them). See THIRD_PARTY.md.
//! Edge-extended sprite borders (vanilla `TextureAtlas.uploadInitialContents`).
/// Writes a sprite's own edge texels into the `border` texels around its
/// content at one mip level (vanilla `TextureAtlas.uploadInitialContents`: each
/// sprite is blitted into its padded slot through a clamp-to-edge sampler, so
/// every border texel — corners included — is a copy of the nearest edge texel
/// of the sprite's own mip level, never of a neighbouring sprite).
/// `content` is the sprite's rect at this level; `level_size` is the atlas side
/// at this level.
pub(super) fn extend_border(pixels: &mut [u8], level_size: u32, content: [u32; 4], border: u32) {
    let [cx, cy, cw, ch] = content;
    if border == 0 || cw == 0 || ch == 0 || cx < border || cy < border {
        return;
    }
    let read = |pixels: &[u8], px: u32, py: u32| -> [u8; 4] {
        let at = ((py * level_size + px) * 4) as usize;
        [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
    };
    // Rows above and below: the content's first / last row, edge-extended
    // sideways (so the corners are the corner texel, like the clamped sample).
    for offset in 0..border {
        for (row, to) in [(cy, cy - border + offset), (cy + ch - 1, cy + ch + offset)] {
            let left = read(pixels, cx, row);
            let right = read(pixels, cx + cw - 1, row);
            for step in 0..border {
                write_texel(pixels, level_size, cx - border + step, to, left);
                write_texel(pixels, level_size, cx + cw + step, to, right);
            }
            for px in 0..cw {
                let color = read(pixels, cx + px, row);
                write_texel(pixels, level_size, cx + px, to, color);
            }
        }
    }
    // Columns beside the content rows.
    for py in 0..ch {
        let left = read(pixels, cx, cy + py);
        let right = read(pixels, cx + cw - 1, cy + py);
        for step in 0..border {
            write_texel(pixels, level_size, cx - border + step, cy + py, left);
            write_texel(pixels, level_size, cx + cw + step, cy + py, right);
        }
    }
}

fn write_texel(pixels: &mut [u8], size: u32, px: u32, py: u32, color: [u8; 4]) {
    let at = ((py * size + px) * 4) as usize;
    pixels[at..at + 4].copy_from_slice(&color);
}
