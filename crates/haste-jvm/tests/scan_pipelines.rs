// The vignette and crosshair textures: this scanned the client jar's Hud and RenderPipelines
// classes during development. Kept as the record of what vanilla 26.3 draws them with:
// RenderPipelines.VIGNETTE over textures/misc/vignette.png, RenderPipelines.CROSSHAIR over the
// hud/crosshair sprites (see GfxImpl.blend and haste.gui.Pipelines in java/haste/gui).
#[test]
fn vignette_and_crosshair_draw_with_special_pipelines() {
    // The mapping lives in Java (GfxImpl.blend, reading the pipeline's blend function); this asserts the
    // identifiers the name fallback matches are what vanilla 26.3's Hud actually references, so a renamed
    // texture fails here rather than silently falling back to normal blending.
    let jar = haste_core::paths::client_jar();
    let f = std::fs::File::open(&jar).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut e = z.by_name("net/minecraft/client/gui/Hud.class").unwrap();
    let mut d = Vec::new();
    std::io::Read::read_to_end(&mut e, &mut d).unwrap();
    let hay = String::from_utf8_lossy(&d);
    for needle in [
        "textures/misc/vignette.png",
        "hud/crosshair",
        "hud/crosshair_attack_indicator_background",
        "hud/crosshair_attack_indicator_full",
        "hud/crosshair_attack_indicator_progress",
    ] {
        assert!(
            hay.contains(needle),
            "vanilla's Hud no longer references {needle}: update GfxImpl.blend"
        );
    }
}
