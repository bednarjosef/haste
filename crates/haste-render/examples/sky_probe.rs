//! Sky probe (dev tool): the mean sRGB colour of horizontal bands of a screenshot, so the
//! dome's colour at a given height can be compared against the expected vanilla value.
//!
//! usage: cargo run --release --example sky_probe -- <shot.png> [rows-per-band]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: sky_probe <shot.png> [rows-per-band]");
        std::process::exit(2);
    }
    let band: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);
    let img = image::open(&args[1]).expect("open").to_rgba8();
    let (w, h) = img.dimensions();
    println!("{} {}x{} (bands of {} rows):", args[1], w, h, band);
    let mut y = 0;
    while y < h {
        let end = (y + band).min(h);
        let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
        for py in y..end {
            for px in 0..w {
                let p = img.get_pixel(px, py);
                r += p[0] as u64;
                g += p[1] as u64;
                b += p[2] as u64;
                n += 1;
            }
        }
        println!(
            "rows {:3}-{:<3}: mean sRGB ({:3}, {:3}, {:3})",
            y,
            end - 1,
            r / n,
            g / n,
            b / n
        );
        y = end;
    }
}
