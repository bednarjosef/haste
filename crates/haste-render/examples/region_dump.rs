// Prints the pixels of a rectangular region of a PNG as hex colours (dev tool for
// inspecting distant-terrain shots without opening them).
// Usage: cargo run -p haste-render --example region_dump -- <file.png> <x0> <y0> <x1> <y1> [step]
use image::GenericImageView;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("png path");
    let x0: u32 = args.next().expect("x0").parse().expect("x0 int");
    let y0: u32 = args.next().expect("y0").parse().expect("y0 int");
    let x1: u32 = args.next().expect("x1").parse().expect("x1 int");
    let y1: u32 = args.next().expect("y1").parse().expect("y1 int");
    let step: u32 = args
        .next()
        .map(|s| s.parse().expect("step int"))
        .unwrap_or(1);
    let img = image::open(&path).expect("open png");
    let (w, h) = img.dimensions();
    let rgb = img.to_rgb8();
    let (x1, y1) = (x1.min(w - 1), y1.min(h - 1));
    println!("{path} region x{x0}..{x1} y{y0}..{y1} step {step}");
    let mut y = y0;
    while y <= y1 {
        let mut row = String::new();
        let mut x = x0;
        while x <= x1 {
            let [r, g, b] = rgb.get_pixel(x, y).0;
            row.push_str(&format!("{r:02x}{g:02x}{b:02x} "));
            x += step;
        }
        println!("y{y:4}: {row}");
        y += step;
    }
}
