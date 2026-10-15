//! Screenshot comparison for debugging (dev tool, like `offscreen_dump`): compares two
//! PNGs pixel by pixel and prints the differing fraction, mean error and the sky region's
//! share, so a rendering change can be checked without opening images.
//!
//! usage: cargo run --release --example weather_diff -- <a.png> <b.png>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: weather_diff <a.png> <b.png>");
        std::process::exit(2);
    }
    let a = image::open(&args[1]).expect("open a").to_rgba8();
    let b = image::open(&args[2]).expect("open b").to_rgba8();
    if a.dimensions() != b.dimensions() {
        eprintln!(
            "size mismatch: {:?} vs {:?}",
            a.dimensions(),
            b.dimensions()
        );
        std::process::exit(1);
    }
    let (w, h) = a.dimensions();
    let mut differ = 0u64;
    let mut sky_diff = 0u64;
    let mut sky_total = 0u64;
    let mut sum = 0u64;
    let total = (w as u64) * (h as u64);
    for ((x, y, pa), (_, _, pb)) in a.enumerate_pixels().zip(b.enumerate_pixels()) {
        let err =
            pa.0.iter()
                .zip(&pb.0)
                .map(|(ca, cb)| (*ca as i32 - *cb as i32).unsigned_abs() as u32)
                .max()
                .unwrap_or(0);
        sum += err as u64;
        let differs = err > 8;
        if y < h / 3 {
            sky_total += 1;
            sky_diff += differs as u64;
        }
        differ += differs as u64;
    }
    println!(
        "{}x{}: {}/{} pixels differ >8 ({:.1}%), mean error {:.2}",
        w,
        h,
        differ,
        total,
        100.0 * differ as f64 / total as f64,
        sum as f64 / total as f64 / 3.0
    );
    println!(
        "sky region (top third): {}/{} differ ({:.1}%)",
        sky_diff,
        sky_total,
        100.0 * sky_diff as f64 / sky_total.max(1) as f64
    );
}
