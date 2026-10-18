//! Player options: the `[options]` section of the config, the counterpart of vanilla's
//! `options.txt`.
//!
//! Defaults mirror vanilla's options.txt defaults (minecraft.wiki "Options.txt" page).
//! Defaults I am not fully certain about, listed here per the brief:
//! - `simulation_distance` 12 (vanilla's default simulation distance; some versions ship 10),
//! - `max_fps` 120 (vanilla's default framerate limit; 260 means "unlimited" there),
//! - `gui_scale` is stored as written, like vanilla's `Options.guiScale` (0 = auto, any
//!   larger value is bounded by the window only when used: `Window.calculateScale`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod cloud_status;

pub use cloud_status::CloudStatus;

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
    /// 0 = auto (vanilla: `Options.AUTO_GUI_SCALE`). Not clamped here: like vanilla's
    /// `Options.guiScale`, the stored value is bounded against the real window only when
    /// it is used (`Window.calculateScale`, `resolve_gui_scale` in the client).
    pub gui_scale: u8,
    /// Vanilla's "Force Unicode Font" option (`Options.forceUnicodeFont`, default off). Also
    /// rounds an odd GUI scale up to the next even one (`Window.calculateScale`).
    pub force_unicode_font: bool,
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
    /// Vanilla's "Entity Distance" scaling option, 0.5..=5.0 (default 1.0). Scales the
    /// size-based entity render cull (`Entity.shouldRenderAtSqrDistance`).
    pub entity_distance_scaling: f32,
    /// Vanilla's "Darkness Pulsing" accessibility option, 0.0..=1.0 (default 1.0). Scales the
    /// darkness effect in the lightmap (`LightmapRenderState.darknessEffectScale`).
    pub darkness_effect_scale: f32,
    /// Vanilla's "Menu Background Blurriness" accessibility option, 0..=10 (default 5). The
    /// box-blur radius the open screen's blurred background uses (`Options.menuBackgroundBlurriness`).
    pub menu_background_blurriness: u8,
    /// 260 means "unlimited", as in vanilla's framerate limit.
    pub max_fps: u32,
    /// Vanilla's "Clouds" video setting (`Options.cloudStatus`, options.txt
    /// `renderClouds`; default FANCY).
    pub clouds: CloudStatus,
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
            force_unicode_font: false,
            fullscreen: false,
            vsync: true,
            bob_view: true,
            fov_effect_scale: 1.0,
            damage_tilt_strength: 1.0,
            gamma: 0.5,
            darkness_effect_scale: 1.0,
            entity_distance_scaling: 1.0,
            menu_background_blurriness: 5,
            max_fps: 120,
            clouds: CloudStatus::default(),
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
        self.fov_effect_scale = self.fov_effect_scale.clamp(0.0, 1.0);
        self.damage_tilt_strength = self.damage_tilt_strength.clamp(0.0, 1.0);
        self.gamma = self.gamma.clamp(0.0, 1.0);
        self.darkness_effect_scale = self.darkness_effect_scale.clamp(0.0, 1.0);
        self.entity_distance_scaling = self.entity_distance_scaling.clamp(0.5, 5.0);
        self.menu_background_blurriness = self.menu_background_blurriness.min(10);
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
            "forceUnicodeFont" => {
                if let Ok(v) = value.parse::<bool>() {
                    self.force_unicode_font = v;
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
            "entityDistanceScaling" => {
                if let Ok(v) = value.parse::<f32>() {
                    self.entity_distance_scaling = v;
                }
                true
            }
            // An IntRange(0, 10) in vanilla; the blur's box-blur radius.
            "menuBackgroundBlurriness" => {
                if let Ok(v) = value.parse::<u8>() {
                    self.menu_background_blurriness = v;
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
            // The clouds mode: options.txt writes vanilla's serialized names
            // ("false"/"fast"/"true"), the live option bridge the enum names.
            "renderClouds" => {
                if let Some(v) = CloudStatus::parse(value) {
                    self.clouds = v;
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
mod tests;
