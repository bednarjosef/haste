// Reports the colour histogram of a PNG: how many pixels look like the haste placeholder
// entity box (grey body ~ (200,200,210) with grid ~ (120,120,130)) versus everything else.
// Usage: cargo run -p haste-render --example png_report -- <file.png>
use image::GenericImageView;

fn main() {
    let path = std::env::args().nth(1).expect("png path");
    let img = image::open(&path).expect("open png");
    let (w, h) = img.dimensions();
    let rgb = img.to_rgb8();
    let mut grid = 0usize;
    let mut body = 0usize;
    let mut tinted_red = 0usize; // red-dominant pixels (a red-dyed wool check, any light)
    let mut white_snow = 0usize; // the snow platform
    for (_, _, px) in rgb.enumerate_pixels() {
        let [r, g, b] = px.0;
        if (r as i32 - 120).abs() < 15 && (g as i32 - 120).abs() < 15 && (b as i32 - 130).abs() < 15
        {
            grid += 1;
        }
        if (r as i32 - 200).abs() < 15 && (g as i32 - 200).abs() < 15 && (b as i32 - 210).abs() < 15
        {
            body += 1;
        }
        if r as i32 - g as i32 > 15 && r as i32 - b as i32 > 15 {
            tinted_red += 1;
        }
        if r > 235 && g > 235 && b > 235 {
            white_snow += 1;
        }
    }
    let total = (w * h) as usize;
    let mut blue = 0usize;
    let mut green = 0usize;
    for (_, _, px) in rgb.enumerate_pixels() {
        let [r, g, b] = px.0;
        if b as i32 - r as i32 > 15 && b as i32 - g as i32 > 10 {
            blue += 1;
        }
        if g as i32 - r as i32 > 15 && g as i32 - b as i32 > 15 {
            green += 1;
        }
    }
    println!(
        "{path} {w}x{h}: grid={} body={} red={} blue={} green={} snow={} total={total}",
        grid, body, tinted_red, blue, green, white_snow
    );
}
