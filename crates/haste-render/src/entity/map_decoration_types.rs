//! `MapDecorationTypes`: the vanilla `minecraft:map_decoration_type` entries' data the map
//! renderer needs — which sprite each type draws (`MapDecorationType.assetId`, a
//! `map/decorations` texture) and whether it also draws on a framed map
//! (`showOnItemFrame`). The registry itself is not synced with data (it is not in
//! `RegistryDataLoader.SYNCHRONIZED_REGISTRIES`), so the client resolves an id to its name
//! through the static registry and the type's fields through this table.
//!
//! An id whose name is missing here (a mod's decoration type, or a newer vanilla entry) has
//! no known sprite: the map draws nothing for it and logs once, never panicking
//! (`map_decoration_tests` checks the table against the decompiled source when the cache
//! exists).

use haste_core::Identifier;

/// One vanilla map decoration type: the sprite asset (under `textures/map/decorations/`)
/// and the item-frame flag.
pub(super) struct MapDecorationType {
    /// The sprite's file under `textures/map/decorations/` (`assetId` without the
    /// `map/decorations/` prefix the atlas's directory source adds).
    pub asset: &'static str,
    /// `showOnItemFrame`.
    pub show_on_frame: bool,
}

/// The vanilla types by registry entry name (`MapDecorationTypes.register(name, assetName,
/// showOnItemFrame, trackCount)`; `trackCount` is server-side bookkeeping only).
#[rustfmt::skip]
pub(super) fn by_registry_name(name: &Identifier) -> Option<MapDecorationType> {
    let asset = match (name.namespace(), name.path()) {
        ("minecraft", "player") => "player",
        ("minecraft", "frame") => "frame",
        ("minecraft", "red_marker") => "red_marker",
        ("minecraft", "blue_marker") => "blue_marker",
        ("minecraft", "target_x") => "target_x",
        ("minecraft", "target_point") => "target_point",
        ("minecraft", "player_off_map") => "player_off_map",
        ("minecraft", "player_off_limits") => "player_off_limits",
        ("minecraft", "mansion") => "woodland_mansion",
        ("minecraft", "monument") => "ocean_monument",
        ("minecraft", "banner_white") => "white_banner",
        ("minecraft", "banner_orange") => "orange_banner",
        ("minecraft", "banner_magenta") => "magenta_banner",
        ("minecraft", "banner_light_blue") => "light_blue_banner",
        ("minecraft", "banner_yellow") => "yellow_banner",
        ("minecraft", "banner_lime") => "lime_banner",
        ("minecraft", "banner_pink") => "pink_banner",
        ("minecraft", "banner_gray") => "gray_banner",
        ("minecraft", "banner_light_gray") => "light_gray_banner",
        ("minecraft", "banner_cyan") => "cyan_banner",
        ("minecraft", "banner_purple") => "purple_banner",
        ("minecraft", "banner_blue") => "blue_banner",
        ("minecraft", "banner_brown") => "brown_banner",
        ("minecraft", "banner_green") => "green_banner",
        ("minecraft", "banner_red") => "red_banner",
        ("minecraft", "banner_black") => "black_banner",
        ("minecraft", "red_x") => "red_x",
        ("minecraft", "village_desert") => "desert_village",
        ("minecraft", "village_plains") => "plains_village",
        ("minecraft", "village_savanna") => "savanna_village",
        ("minecraft", "village_snowy") => "snowy_village",
        ("minecraft", "village_taiga") => "taiga_village",
        ("minecraft", "jungle_temple") => "jungle_temple",
        ("minecraft", "swamp_hut") => "swamp_hut",
        ("minecraft", "trial_chambers") => "trial_chambers",
        ("minecraft", "abandoned_camp") => "abandoned_camp",
        ("minecraft", "ancient_city") => "ancient_city",
        ("minecraft", "desert_pyramid") => "desert_pyramid",
        ("minecraft", "mineshaft") => "mineshaft",
        ("minecraft", "ocean_ruin_warm") => "warm_ocean_ruins",
        _ => return None,
    };
    let show_on_frame = !matches!(
        (name.namespace(), name.path()),
        ("minecraft", "player")
            | ("minecraft", "red_marker")
            | ("minecraft", "blue_marker")
            | ("minecraft", "player_off_map")
            | ("minecraft", "player_off_limits")
    );
    Some(MapDecorationType {
        asset,
        show_on_frame,
    })
}
