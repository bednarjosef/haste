//! Player options: the `[options]` section of the config, the counterpart of vanilla's
//! `options.txt`.
//!
//! Defaults mirror vanilla's options.txt defaults (minecraft.wiki "Options.txt" page).
//! Defaults I am not fully certain about, listed here per the brief:
//! - `simulation_distance` 12 (vanilla's default simulation distance; some versions ship 10),
//! - `max_fps` 120 (vanilla's default framerate limit; 260 means "unlimited" there),
//! - `gui_scale` is clamped to 0..=4; vanilla added the 4 ("large") step only in recent
//!   versions, older ones cap at 3.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Per-category sound volumes, each 0.0..=1.0. Category names follow vanilla's
/// `soundCategory_*` keys.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SoundVolumes {
    #[serde(default = "one")]
    pub master: f32,
    #[serde(default = "one")]
    pub music: f32,
    #[serde(default = "one")]
    pub record: f32,
    #[serde(default = "one")]
    pub weather: f32,
    #[serde(default = "one")]
    pub block: f32,
    #[serde(default = "one")]
    pub hostile: f32,
    #[serde(default = "one")]
    pub neutral: f32,
    #[serde(default = "one")]
    pub player: f32,
    #[serde(default = "one")]
    pub ambient: f32,
    #[serde(default = "one")]
    pub voice: f32,
    #[serde(default = "one")]
    pub ui: f32,
}

fn one() -> f32 {
    1.0
}

impl Default for SoundVolumes {
    fn default() -> Self {
        SoundVolumes {
            master: 1.0,
            music: 1.0,
            record: 1.0,
            weather: 1.0,
            block: 1.0,
            hostile: 1.0,
            neutral: 1.0,
            player: 1.0,
            ambient: 1.0,
            voice: 1.0,
            ui: 1.0,
        }
    }
}

impl SoundVolumes {
    fn clamped(&mut self) {
        let all = [
            &mut self.master,
            &mut self.music,
            &mut self.record,
            &mut self.weather,
            &mut self.block,
            &mut self.hostile,
            &mut self.neutral,
            &mut self.player,
            &mut self.ambient,
            &mut self.voice,
            &mut self.ui,
        ];
        for volume in all {
            *volume = volume.clamp(0.0, 1.0);
        }
    }
}

/// Keybindings: action name -> vanilla key name, e.g. `"key.forward" -> "key.keyboard.w"`.
/// Actions present in a config file that haste does not know are kept, not dropped.
/// Action `key.playerlist` is vanilla's `key_key.playerList` with the casing normalized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keybinds {
    #[serde(flatten)]
    pub bindings: BTreeMap<String, String>,
}

impl Keybinds {
    /// Vanilla's default bindings.
    pub fn vanilla() -> Keybinds {
        let bindings: &[(&str, &str)] = &[
            ("key.forward", "key.keyboard.w"),
            ("key.left", "key.keyboard.a"),
            ("key.back", "key.keyboard.s"),
            ("key.right", "key.keyboard.d"),
            ("key.jump", "key.keyboard.space"),
            ("key.sneak", "key.keyboard.left.shift"),
            ("key.sprint", "key.keyboard.left.control"),
            ("key.inventory", "key.keyboard.e"),
            ("key.drop", "key.keyboard.q"),
            ("key.chat", "key.keyboard.t"),
            ("key.command", "key.keyboard.slash"),
            ("key.hotbar.1", "key.keyboard.1"),
            ("key.hotbar.2", "key.keyboard.2"),
            ("key.hotbar.3", "key.keyboard.3"),
            ("key.hotbar.4", "key.keyboard.4"),
            ("key.hotbar.5", "key.keyboard.5"),
            ("key.hotbar.6", "key.keyboard.6"),
            ("key.hotbar.7", "key.keyboard.7"),
            ("key.hotbar.8", "key.keyboard.8"),
            ("key.hotbar.9", "key.keyboard.9"),
            ("key.playerlist", "key.keyboard.tab"),
            ("key.screenshot", "key.keyboard.f2"),
            ("key.togglePerspective", "key.keyboard.f5"),
            ("key.fullscreen", "key.keyboard.f11"),
            ("key.swapOffhand", "key.keyboard.f"),
            ("key.attack", "key.mouse.left"),
            ("key.use", "key.mouse.right"),
            ("key.pickItem", "key.mouse.middle"),
        ];
        Keybinds {
            bindings: bindings
                .iter()
                .map(|(action, key)| (action.to_string(), key.to_string()))
                .collect(),
        }
    }

    /// The key bound to `action`, if any.
    pub fn get(&self, action: &str) -> Option<&str> {
        self.bindings.get(action).map(String::as_str)
    }
}

impl Default for Keybinds {
    fn default() -> Self {
        Keybinds::vanilla()
    }
}

/// Saved player options, like vanilla's options.txt. Ranges are enforced on load by
/// [`Options::sanitized`], not by serde, so a hand-edited file still loads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Options {
    pub render_distance: u8,
    pub simulation_distance: u8,
    pub fov: u8,
    /// 0.0..=1.0 (vanilla stores 0..=1 in current versions; pre-1.18 it was 0..=0.5).
    pub mouse_sensitivity: f32,
    /// 0 = auto.
    pub gui_scale: u8,
    pub fullscreen: bool,
    pub vsync: bool,
    /// Vanilla's "View Bobbing" accessibility option (default on).
    pub bob_view: bool,
    /// Vanilla's "FOV Effects" accessibility option, 0.0..=1.0 (default 1.0).
    pub fov_effect_scale: f32,
    /// Vanilla's "Damage Tilt Strength" accessibility option, 0.0..=1.0 (default 1.0).
    pub damage_tilt_strength: f32,
    /// Vanilla's "Brightness" (gamma) slider, 0.0..=1.0 (default 0.5). Feeds the lightmap's
    /// brightness factor (`LightmapRenderState.brightness`).
    pub gamma: f32,
    /// Vanilla's "Darkness Pulsing" accessibility option, 0.0..=1.0 (default 1.0). Scales the
    /// darkness effect in the lightmap (`LightmapRenderState.darknessEffectScale`).
    pub darkness_effect_scale: f32,
    /// 260 means "unlimited", as in vanilla's framerate limit.
    pub max_fps: u32,
    pub sound: SoundVolumes,
    pub keys: Keybinds,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            render_distance: 12,
            simulation_distance: 12,
            fov: 70,
            mouse_sensitivity: 0.5,
            gui_scale: 0,
            fullscreen: false,
            vsync: true,
            bob_view: true,
            fov_effect_scale: 1.0,
            damage_tilt_strength: 1.0,
            gamma: 0.5,
            darkness_effect_scale: 1.0,
            max_fps: 120,
            sound: SoundVolumes::default(),
            keys: Keybinds::default(),
        }
    }
}

impl Options {
    /// Returns a copy with out-of-range values clamped into range and known keybinds
    /// missing from the file restored to their vanilla defaults. Applied on load so a
    /// hand-edited or partial config never yields unusable settings.
    pub fn sanitized(mut self) -> Options {
        self.render_distance = self.render_distance.clamp(2, 32);
        self.simulation_distance = self.simulation_distance.clamp(2, 32);
        self.fov = self.fov.clamp(30, 110);
        self.mouse_sensitivity = self.mouse_sensitivity.clamp(0.0, 1.0);
        self.gui_scale = self.gui_scale.clamp(0, 4);
        self.fov_effect_scale = self.fov_effect_scale.clamp(0.0, 1.0);
        self.damage_tilt_strength = self.damage_tilt_strength.clamp(0.0, 1.0);
        self.gamma = self.gamma.clamp(0.0, 1.0);
        self.darkness_effect_scale = self.darkness_effect_scale.clamp(0.0, 1.0);
        self.max_fps = self.max_fps.clamp(10, 260);
        self.sound.clamped();
        for (action, key) in &Keybinds::vanilla().bindings {
            self.keys
                .bindings
                .entry(action.clone())
                .or_insert_with(|| key.clone());
        }
        self
    }

    /// Applies one vanilla `options.txt` entry (`key:value`) to `options.txt`'s own fields.
    /// Returns whether the key is one haste uses. Malformed values leave the field
    /// untouched, like vanilla's load skipping them.
    pub fn apply_one(&mut self, key: &str, value: &str) -> bool {
        match key {
            "renderDistance" => {
                if let Ok(v) = value.parse::<u8>() {
                    self.render_distance = v;
                }
                true
            }
            "simulationDistance" => {
                if let Ok(v) = value.parse::<u8>() {
                    self.simulation_distance = v;
                }
                true
            }
            "fov" => {
                if let Ok(v) = value.parse::<f32>() {
                    // vanilla's option value is degrees (30..=110, "Normal" 70); options.txt
                    // stores the normalized slider 0.0..=1.0 that 26.3 maps to 70°..110°
                    // (`Codec.DOUBLE.xmap(v -> v * 40 + 70, ...)`, `Options.fov`).
                    self.fov = if v <= 1.0 { 70.0 + v * 40.0 } else { v } as u8;
                }
                true
            }
            "mouseSensitivity" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.mouse_sensitivity = v;
                }
                true
            }
            "guiScale" => {
                if let Ok(v) = value.parse::<u8>() {
                    self.gui_scale = v;
                }
                true
            }
            "fullscreen" => {
                if let Ok(v) = value.parse::<bool>() {
                    self.fullscreen = v;
                }
                true
            }
            "vsync" | "enableVsync" => {
                if let Ok(v) = value.parse::<bool>() {
                    self.vsync = v;
                }
                true
            }
            "bobView" => {
                if let Ok(v) = value.parse::<bool>() {
                    self.bob_view = v;
                }
                true
            }
            // Vanilla's option codec squares the slider value before storing it, so the
            // effective scale is the square root of what is in the file.
            "fovEffectScale" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.fov_effect_scale = v.sqrt();
                }
                true
            }
            "damageTiltStrength" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.damage_tilt_strength = v;
                }
                true
            }
            "gamma" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.gamma = v;
                }
                true
            }
            // The darkness option's codec squares the slider value on load (the same
            // `xmap(Mth::square, sqrt)` as fovEffectScale, but with no explicit raw codec),
            // so the effective scale is the square of the file value.
            "darknessEffectScale" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.darkness_effect_scale = v * v;
                }
                true
            }
            // framerateLimit is the current name, maxFps the pre-1.20.5 one.
            "framerateLimit" | "maxFps" => {
                if let Ok(v) = value.parse::<u32>() {
                    self.max_fps = v;
                }
                true
            }
            _ => {
                if let Some(action) = key.strip_prefix("soundCategory_") {
                    let volume = match value.parse::<f32>() {
                        Ok(v) => v.clamp(0.0, 1.0),
                        Err(_) => return true,
                    };
                    match action {
                        "master" => self.sound.master = volume,
                        "music" => self.sound.music = volume,
                        "record" => self.sound.record = volume,
                        "weather" => self.sound.weather = volume,
                        "block" => self.sound.block = volume,
                        "hostile" => self.sound.hostile = volume,
                        "neutral" => self.sound.neutral = volume,
                        "player" => self.sound.player = volume,
                        "ambient" => self.sound.ambient = volume,
                        "voice" => self.sound.voice = volume,
                        "ui" => self.sound.ui = volume,
                        _ => {}
                    }
                    true
                } else if let Some(action) = key.strip_prefix("key_key.") {
                    // Normalize vanilla's playerList casing to haste's key.playerlist.
                    let action = if action == "playerList" {
                        "playerlist"
                    } else {
                        action
                    };
                    self.keys
                        .bindings
                        .insert(format!("key.{action}"), value.to_owned());
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Parses a vanilla `options.txt` (`key:value` lines) into `Options`, starting from
    /// defaults. Keys haste does not use are ignored; malformed lines are skipped rather
    /// than failing the import; values are clamped via [`Options::sanitized`]. Keybind
    /// entries (`key_key.*`) are taken as-is, including ones haste does not know.
    pub fn import_vanilla(options_txt: &str) -> Options {
        let mut opts = Options::default();
        for line in options_txt.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            opts.apply_one(key, value);
        }
        opts.sanitized()
    }
}

#[cfg(test)]
mod tests {
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
    fn roundtrips_through_toml() {
        let opts = Options {
            render_distance: 20,
            simulation_distance: 8,
            fov: 90,
            mouse_sensitivity: 0.25,
            gui_scale: 3,
            fullscreen: true,
            vsync: false,
            bob_view: false,
            fov_effect_scale: 0.5,
            damage_tilt_strength: 0.75,
            gamma: 0.25,
            darkness_effect_scale: 0.5,
            max_fps: 60,
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
        assert_eq!(opts.gui_scale, 4);
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
}
