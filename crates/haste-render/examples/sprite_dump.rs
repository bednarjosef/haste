// Prints a sprite's texels from the block atlas as built from the client jar
// (dev tool for inspecting textures without an image viewer).
// Usage: cargo run -p haste-render --example sprite_dump -- <texture path> [mip level]
fn main() {
    let mut args = std::env::args().skip(1);
    let texture = args
        .next()
        .expect("texture path, e.g. minecraft:block/snow");
    let mip: u32 = args
        .next()
        .map(|m| m.parse().expect("mip int"))
        .unwrap_or(0);
    let jar = haste_core::paths::client_jar();
    let mut assets =
        haste_assets::Assets::open(&jar, &haste_core::paths::assets_dir(), "26.3").unwrap();
    let states = haste_assets::BlocksJsonStates::load(
        &haste_core::paths::datagen_reports().join("blocks.json"),
    )
    .unwrap();
    let mut atlas = haste_assets::AtlasBuilder::build(&mut assets, &[], false).unwrap();
    let _ = haste_assets::BakedBlockModels::bake(&mut assets, &states, &mut atlas).unwrap();
    let region = atlas.region(&texture);
    let [x, y, w, h] = region.pixel_rect;
    let (x, y, w, h) = (x as usize, y as usize, w as usize, h as usize);
    let size = (atlas.size() >> mip) as usize;
    let pixels = atlas.pixels();
    println!(
        "{texture}: region {x},{y} {w}x{h} opaque={} (level {mip}, atlas {}x{})",
        region.opaque, size, size
    );
    // The concatenated mip chain: level `mip` starts after all smaller levels.
    let mut offset = 0usize;
    for level in 0..mip {
        let d = (atlas.size() >> level) as usize;
        offset += d * d * 4;
    }
    let base = offset + ((y >> mip) * size + (x >> mip)) * 4;
    for row in 0..(h >> mip) {
        let mut line = String::new();
        for col in 0..(w >> mip) {
            let at = base + row * size * 4 + col * 4;
            line.push_str(&format!(
                "{:02x}{:02x}{:02x} ",
                pixels[at],
                pixels[at + 1],
                pixels[at + 2]
            ));
        }
        println!("{line}");
    }
}
