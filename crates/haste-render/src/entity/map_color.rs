//! `MapColor`: the 64-entry map palette and the four brightness levels vanilla's map
//! textures pack pixel colours through (`MapColor.getColorFromPackedId`).
//!
//! The table is vanilla's own constant list (it hardcodes it in `MapColor`, so no data
//! generator or jar asset carries it); `map_color_tests` verifies every entry against the
//! decompiled 26.3 source in the cache when it exists.

/// The brightness levels of a packed map pixel (`MapColor.Brightness`), indexed by the
/// brightness id: LOW, NORMAL, HIGH, LOWEST (LOWEST has id 3). Each scales the palette
/// colour's channels by `channel * modifier / 255` (`ARGB.scaleRGB`).
const BRIGHTNESS: [u32; 4] = [180, 220, 255, 135];

/// The base colours by map colour id 0..63 (`MapColor`'s own table; `NONE` is id 0 and draws
/// fully transparent whatever its brightness). Unused ids are 0. Vanilla source: `MapColor`.
#[rustfmt::skip]
pub(super) const COLORS: [u32; 64] = [
    0,        // 0 NONE
    8368696,  // 1 GRASS
    16247203, // 2 SAND
    13092807, // 3 WOOL
    16711680, // 4 FIRE
    10526975, // 5 ICE
    10987431, // 6 METAL
    31744,    // 7 PLANT
    16777215, // 8 SNOW
    10791096, // 9 CLAY
    9923917,  // 10 DIRT
    7368816,  // 11 STONE
    4210943,  // 12 WATER
    9402184,  // 13 WOOD
    16776437, // 14 QUARTZ
    14188339, // 15 COLOR_ORANGE
    11685080, // 16 COLOR_MAGENTA
    6724056,  // 17 COLOR_LIGHT_BLUE
    15066419, // 18 COLOR_YELLOW
    8375321,  // 19 COLOR_LIGHT_GREEN
    15892389, // 20 COLOR_PINK
    5000268,  // 21 COLOR_GRAY
    10066329, // 22 COLOR_LIGHT_GRAY
    5013401,  // 23 COLOR_CYAN
    8339378,  // 24 COLOR_PURPLE
    3361970,  // 25 COLOR_BLUE
    6704179,  // 26 COLOR_BROWN
    6717235,  // 27 COLOR_GREEN
    10040115, // 28 COLOR_RED
    1644825,  // 29 COLOR_BLACK
    16445005, // 30 GOLD
    6085589,  // 31 DIAMOND
    4882687,  // 32 LAPIS
    55610,    // 33 EMERALD
    8476209,  // 34 PODZOL
    7340544,  // 35 NETHER
    13742497, // 36 TERRACOTTA_WHITE
    10441252, // 37 TERRACOTTA_ORANGE
    9787244,  // 38 TERRACOTTA_MAGENTA
    7367818,  // 39 TERRACOTTA_LIGHT_BLUE
    12223780, // 40 TERRACOTTA_YELLOW
    6780213,  // 41 TERRACOTTA_LIGHT_GREEN
    10505550, // 42 TERRACOTTA_PINK
    3746083,  // 43 TERRACOTTA_GRAY
    8874850,  // 44 TERRACOTTA_LIGHT_GRAY
    5725276,  // 45 TERRACOTTA_CYAN
    8014168,  // 46 TERRACOTTA_PURPLE
    4996700,  // 47 TERRACOTTA_BLUE
    4993571,  // 48 TERRACOTTA_BROWN
    5001770,  // 49 TERRACOTTA_GREEN
    9321518,  // 50 TERRACOTTA_RED
    2430480,  // 51 TERRACOTTA_BLACK
    12398641, // 52 CRIMSON_NYLIUM
    9715553,  // 53 CRIMSON_STEM
    6035741,  // 54 CRIMSON_HYPHAE
    1474182,  // 55 WARPED_NYLIUM
    3837580,  // 56 WARPED_STEM
    5647422,  // 57 WARPED_HYPHAE
    1356933,  // 58 WARPED_WART_BLOCK
    6579300,  // 59 DEEPSLATE
    14200723, // 60 RAW_IRON
    8365974,  // 61 GLOW_LICHEN
    0,        // 62 (unused)
    0,        // 63 (unused)
];

/// `MapColor.getColorFromPackedId`: the RGBA colour of one packed map pixel — the palette
/// colour of its id nibble (unknown ids are `NONE`, transparent) scaled by its brightness
/// nibble's modifier (`ARGB.scaleRGB`: `channel * modifier / 255`). `NONE` is fully
/// transparent, everything else opaque.
pub(super) fn color_from_packed_id(packed: u8) -> [u8; 4] {
    let id = (packed >> 2) as usize & 0x3F;
    if id == 0 {
        return [0, 0, 0, 0];
    }
    let m = BRIGHTNESS[(packed & 3) as usize];
    let c = COLORS[id];
    [
        (((c >> 16) & 0xFF) * m / 255) as u8,
        (((c >> 8) & 0xFF) * m / 255) as u8,
        ((c & 0xFF) * m / 255) as u8,
        255,
    ]
}
