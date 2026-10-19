use super::*;

#[test]
fn defaults_match_vanilla() {
    let opts = Options::default();
    assert_eq!(opts.render_distance, 12);
    assert_eq!(opts.simulation_distance, 12);
    assert_eq!(opts.fov, 70);
    assert_eq!(opts.mouse_sensitivity, 0.5);
    assert_eq!(opts.gui_scale, 0);
    assert!(!opts.fullscreen);
    assert!(opts.vsync);
    assert_eq!(opts.max_fps, 120);
    assert_eq!(opts.sound, SoundVolumes::default());
    assert_eq!(opts.sound.master, 1.0);
    assert_eq!(opts.sound.ui, 1.0);
    assert_eq!(opts.keys.get("key.forward"), Some("key.keyboard.w"));
    assert_eq!(opts.keys.get("key.sneak"), Some("key.keyboard.left.shift"));
    assert_eq!(opts.keys.get("key.attack"), Some("key.mouse.left"));
    assert_eq!(opts.keys.get("key.use"), Some("key.mouse.right"));
    assert_eq!(opts.keys.get("key.pickItem"), Some("key.mouse.middle"));
    assert_eq!(opts.keys.get("key.hotbar.1"), Some("key.keyboard.1"));
    assert_eq!(opts.keys.get("key.hotbar.9"), Some("key.keyboard.9"));
    assert_eq!(opts.keys.get("key.playerlist"), Some("key.keyboard.tab"));
    assert_eq!(opts.keys.get("key.swapOffhand"), Some("key.keyboard.f"));
    assert!(opts.bob_view);
    assert_eq!(opts.fov_effect_scale, 1.0);
    assert_eq!(opts.damage_tilt_strength, 1.0);
    assert_eq!(opts.gamma, 0.5);
    assert_eq!(opts.darkness_effect_scale, 1.0);
    assert_eq!(opts.menu_background_blurriness, 5);
}

#[test]
fn camera_options_clamp_into_range() {
    let opts = Options {
        fov_effect_scale: 4.0,
        damage_tilt_strength: -2.0,
        ..Options::default()
    }
    .sanitized();
    assert_eq!(opts.fov_effect_scale, 1.0);
    assert_eq!(opts.damage_tilt_strength, 0.0);
}

#[test]
fn camera_options_import_from_vanilla_options_txt() {
    // fovEffectScale is stored squared by vanilla's option codec: 0.25 in the file is a
    // 0.5 effective scale.
    let opts = Options::import_vanilla(
        "bobView:false\nfovEffectScale:0.25\ndamageTiltStrength:0.5\nfov:75\n",
    );
    assert!(!opts.bob_view);
    assert!((opts.fov_effect_scale - 0.5).abs() < 1e-9);
    assert_eq!(opts.damage_tilt_strength, 0.5);
    assert_eq!(opts.fov, 75);
}

#[test]
fn lightmap_options_import_from_vanilla_options_txt() {
    // gamma is the raw 0..1 slider; darknessEffectScale's codec squares the file value.
    let opts = Options::import_vanilla("gamma:0.8\ndarknessEffectScale:0.5\n");
    assert!((opts.gamma - 0.8).abs() < 1e-6);
    assert!((opts.darkness_effect_scale - 0.25).abs() < 1e-6);
    // Clamped like the vanilla range.
    let opts = Options {
        gamma: 2.0,
        darkness_effect_scale: -1.0,
        ..Options::default()
    }
    .sanitized();
    assert_eq!(opts.gamma, 1.0);
    assert_eq!(opts.darkness_effect_scale, 0.0);
}

#[test]
fn menu_blurriness_option_maps_vanilla_radius() {
    // The blur's radius is the option as-is (default 5, IntRange 0..=10); the blur chain
    // only runs from 1 up (Screen.extractBlurredBackground).
    let opts = Options::import_vanilla("menuBackgroundBlurriness:8\n");
    assert_eq!(opts.menu_background_blurriness, 8);
    // Out-of-range values clamp into the vanilla range on load.
    let opts = Options {
        menu_background_blurriness: 11,
        ..Options::default()
    }
    .sanitized();
    assert_eq!(opts.menu_background_blurriness, 10);
}

#[test]
fn entity_distance_option_imports_and_clamps() {
    let opts = Options::import_vanilla("entityDistanceScaling:2.5\n");
    assert!((opts.entity_distance_scaling - 2.5).abs() < 1e-6);
    let opts = Options {
        entity_distance_scaling: 9.0,
        ..Options::default()
    }
    .sanitized();
    assert_eq!(opts.entity_distance_scaling, 5.0);
    let opts = Options {
        entity_distance_scaling: 0.1,
        ..Options::default()
    }
    .sanitized();
    assert_eq!(opts.entity_distance_scaling, 0.5);
}

#[test]
fn roundtrips_through_toml() {
    let opts = Options {
        render_distance: 20,
        simulation_distance: 8,
        fov: 90,
        mouse_sensitivity: 0.25,
        gui_scale: 3,
        force_unicode_font: true,
        fullscreen: true,
        vsync: false,
        bob_view: false,
        fov_effect_scale: 0.5,
        damage_tilt_strength: 0.75,
        gamma: 0.25,
        darkness_effect_scale: 0.5,
        entity_distance_scaling: 1.25,
        menu_background_blurriness: 7,
        max_fps: 60,
        clouds: CloudStatus::Fast,
        cloud_range: 64,
        sound: SoundVolumes {
            master: 0.8,
            music: 0.0,
            ..SoundVolumes::default()
        },
        keys: Keybinds::vanilla(),
    };
    let text = toml::to_string_pretty(&opts).unwrap();
    let back: Options = toml::from_str(&text).unwrap();
    assert_eq!(back, opts);
}

#[test]
fn missing_options_section_gives_defaults() {
    // A Config without [options] (the pre-options file shape) loads with defaults.
    let text = r#"
username = "Steve"
render_distance = 4
gui_scale = 1
fov = 70.0
"#;
    let cfg: crate::config::Config = toml::from_str(text).unwrap();
    assert_eq!(cfg.options, Options::default());
}

#[test]
fn partial_options_section_gives_defaults() {
    let text = "render_distance = 24\n";
    let opts: Options = toml::from_str(text).unwrap();
    assert_eq!(opts.render_distance, 24);
    assert_eq!(opts.fov, 70);
    assert_eq!(opts.sound.music, 1.0);
    assert_eq!(opts.keys.get("key.forward"), Some("key.keyboard.w"));
}

#[test]
fn clamps_out_of_range_values() {
    let opts = Options {
        render_distance: 99,
        simulation_distance: 0,
        fov: 200,
        mouse_sensitivity: 4.0,
        gui_scale: 9,
        max_fps: 1000,
        sound: SoundVolumes {
            master: 2.0,
            music: -1.0,
            ..SoundVolumes::default()
        },
        keys: Keybinds::default(),
        ..Options::default()
    }
    .sanitized();
    assert_eq!(opts.render_distance, 32);
    assert_eq!(opts.simulation_distance, 2);
    assert_eq!(opts.fov, 110);
    assert_eq!(opts.mouse_sensitivity, 1.0);
    // The GUI scale is not clamped on load (vanilla keeps the stored value and bounds
    // it against the window when used), so 9 survives for a 4K window.
    assert_eq!(opts.gui_scale, 9);
    assert_eq!(opts.max_fps, 260);
    assert_eq!(opts.sound.master, 1.0);
    assert_eq!(opts.sound.music, 0.0);
}

#[test]
fn keeps_unknown_keybinds_and_restores_missing_ones() {
    let text = r#"
[keys]
"key.forward" = "key.keyboard.up"
"key.modded.zoom" = "key.keyboard.c"
"#;
    let opts: Options = toml::from_str(text).unwrap();
    // Unknown action kept verbatim, not dropped.
    assert_eq!(opts.keys.get("key.modded.zoom"), Some("key.keyboard.c"));
    assert_eq!(opts.keys.get("key.forward"), Some("key.keyboard.up"));
    // Known actions missing from the file are restored by sanitized().
    assert_eq!(opts.keys.get("key.jump"), None);
    let opts = opts.sanitized();
    assert_eq!(opts.keys.get("key.jump"), Some("key.keyboard.space"));
    assert_eq!(opts.keys.get("key.modded.zoom"), Some("key.keyboard.c"));
}

#[test]
fn cloud_option_imports_and_applies_live() {
    // Vanilla's default is FANCY.
    assert_eq!(Options::default().clouds, CloudStatus::Fancy);
    // options.txt import: the serialized names (`renderClouds:"false"` is what
    // scripts/vanilla-options.sh writes for the reference runs).
    let opts = Options::import_vanilla("renderClouds:false\n");
    assert_eq!(opts.clouds, CloudStatus::Off);
    let opts = Options::import_vanilla("renderClouds:fast\n");
    assert_eq!(opts.clouds, CloudStatus::Fast);
    let opts = Options::import_vanilla("renderClouds:true\n");
    assert_eq!(opts.clouds, CloudStatus::Fancy);
    // The live bridge reports the enum names; malformed values leave it untouched.
    let mut opts = Options::import_vanilla("renderClouds:fast\n");
    assert!(opts.apply_one("renderClouds", "OFF"));
    assert_eq!(opts.clouds, CloudStatus::Off);
    assert!(opts.apply_one("renderClouds", "FANCY"));
    assert_eq!(opts.clouds, CloudStatus::Fancy);
    assert!(opts.apply_one("renderClouds", "maybe"));
    assert_eq!(opts.clouds, CloudStatus::Fancy);
}

#[test]
fn cloud_range_option_imports_clamps_and_applies_live() {
    // Vanilla's default is 128 chunks.
    assert_eq!(Options::default().cloud_range, 128);
    // options.txt import.
    let opts = Options::import_vanilla("cloudRange:32\n");
    assert_eq!(opts.cloud_range, 32);
    // The live bridge reports vanilla's own key; malformed values leave it untouched.
    let mut opts = Options::import_vanilla("cloudRange:32\n");
    assert!(opts.apply_one("cloudRange", "8"));
    assert_eq!(opts.cloud_range, 8);
    assert!(opts.apply_one("cloudRange", "maybe"));
    assert_eq!(opts.cloud_range, 8);
    // Out-of-range values clamp on sanitized() (vanilla's IntRange(2, 128)).
    let opts = Options::import_vanilla("cloudRange:1\n").sanitized();
    assert_eq!(opts.cloud_range, 2);
    let opts = Options::import_vanilla("cloudRange:999\n").sanitized();
    assert_eq!(opts.cloud_range, 128);
}

#[test]
fn apply_one_maps_vanilla_keys_live() {
    // The option bridge reports vanilla's own keys; apply_one is the single mapping
    // both the options.txt import and the live bridge go through.
    let mut opts = Options::default();
    assert!(opts.apply_one("fov", "0.75"));
    assert_eq!(opts.fov, 100);
    assert!(opts.apply_one("fov", "90"));
    assert_eq!(opts.fov, 90);
    assert!(opts.apply_one("renderDistance", "14"));
    assert_eq!(opts.render_distance, 14);
    assert!(opts.apply_one("mouseSensitivity", "0.25"));
    assert!((opts.mouse_sensitivity - 0.25).abs() < 1e-6);
    assert!(opts.apply_one("soundCategory_master", "0.8"));
    assert!((opts.sound.master - 0.8).abs() < 1e-6);
    assert!(opts.apply_one("enableVsync", "false"));
    assert!(!opts.vsync);
    assert!(opts.apply_one("key_key.forward", "key.keyboard.up"));
    assert_eq!(opts.keys.get("key.forward"), Some("key.keyboard.up"));
    assert!(opts.apply_one("key_key.playerList", "key.keyboard.n"));
    assert_eq!(opts.keys.get("key.playerlist"), Some("key.keyboard.n"));
    // The accessibility/video options also apply through the same mapping.
    assert!(opts.apply_one("gamma", "1.0"));
    assert!((opts.gamma - 1.0).abs() < 1e-6);
    assert!(opts.apply_one("bobView", "false"));
    assert!(!opts.bob_view);
    assert!(opts.apply_one("forceUnicodeFont", "true"));
    assert!(opts.force_unicode_font);
    // Unknown keys are ignored, not fatal.
    assert!(!opts.apply_one("skipMultiplayerWarning", "true"));
    // Malformed values leave the field untouched.
    assert!(opts.apply_one("renderDistance", "abc"));
    assert_eq!(opts.render_distance, 14);
}

#[test]
fn imports_vanilla_options_txt() {
    let text = "\
version:4574
renderDistance:8
simulationDistance:6
fov:0.75
mouseSensitivity:0.25
guiScale:2
fullscreen:false
vsync:true
framerateLimit:60
soundCategory_master:0.8
soundCategory_music:0.0
soundCategory_ui:1.0
key_key.forward:key.keyboard.up
key_key.playerList:key.keyboard.n
key_key.hotbar.3:key.keyboard.c
key_key.modded.zoom:key.keyboard.j
gamma:1.0
skipMultiplayerWarning:true
";
    let opts = Options::import_vanilla(text);
    assert_eq!(opts.render_distance, 8);
    assert_eq!(opts.simulation_distance, 6);
    assert_eq!(opts.fov, 100);
    assert_eq!(opts.mouse_sensitivity, 0.25);
    assert_eq!(opts.gui_scale, 2);
    assert!(!opts.fullscreen);
    assert!(opts.vsync);
    assert_eq!(opts.max_fps, 60);
    assert_eq!(opts.sound.master, 0.8);
    assert_eq!(opts.sound.music, 0.0);
    assert_eq!(opts.sound.ui, 1.0);
    assert_eq!(opts.keys.get("key.forward"), Some("key.keyboard.up"));
    // vanilla's playerList maps to haste's key.playerlist
    assert_eq!(opts.keys.get("key.playerlist"), Some("key.keyboard.n"));
    assert_eq!(opts.keys.get("key.hotbar.3"), Some("key.keyboard.c"));
    // unknown keybind actions are imported and kept
    assert_eq!(opts.keys.get("key.modded.zoom"), Some("key.keyboard.j"));
    // gamma is imported too; unknown keys (warnings) are ignored
    assert_eq!(opts.gamma, 1.0);
    assert_eq!(
        opts.keys.get("key.sprint"),
        Some("key.keyboard.left.control")
    );
}
