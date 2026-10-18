// Crops a rectangle out of a PNG, magnified 2x (dev tool for evidence shots).
// Usage: cargo run -p haste-render --example crop_png -- <in.png> <out.png> <x0> <y0> <x1> <y1>
fn main() {
    let mut args = std::env::args().skip(1);
    let input = args.next().expect("input png");
    let output = args.next().expect("output png");
    let x0: u32 = args.next().expect("x0").parse().expect("x0 int");
    let y0: u32 = args.next().expect("y0").parse().expect("y0 int");
    let x1: u32 = args.next().expect("x1").parse().expect("x1 int");
    let y1: u32 = args.next().expect("y1").parse().expect("y1 int");
    let img = image::open(&input).expect("open png");
    let crop = img.crop_imm(x0, y0, x1 - x0, y1 - y0);
    let scaled = crop.resize_exact((x1 - x0) * 2, (y1 - y0) * 2, image::imageops::FilterType::Nearest);
    scaled.save(&output).expect("save png");
    println!("{output}: crop {x0},{y0}..{x1},{y1} at 2x");
}
