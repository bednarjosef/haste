//! What the render thread needs each frame of the local player and its hand (the per-tick
//! "view state" copied out under a short lock): the first-person hand's per-tick animation
//! state, and the rain-fog helpers the tick's view update reads. The own third-person model
//! lives in [`super::player_model`] (its own `haste-entity` state, not a copy here).

use std::time::Instant;

use glam::DVec3;
use haste_core::entity::Pose;
use haste_core::view::BlockView;
use haste_core::{BlockPos, Direction, RegistryIndex};
use haste_protocol::haste_core::game::GameMode;

use super::camera::eye_in_lava;
use super::weather::{biome_has_precipitation, rain_fog_sky_light};

/// The local player's pose for its own model: the physics pose's render equivalent (sleeping and
/// other metadata-only poses are not modelled yet).
pub(super) fn player_pose(s: &crate::net::Session) -> Option<Pose> {
    use haste_physics::Pose as PhysicsPose;
    match s.player.state.pose {
        PhysicsPose::Standing => None,
        PhysicsPose::Crouching => Some(Pose::Crouching),
        PhysicsPose::Swimming => Some(Pose::Swimming),
        PhysicsPose::FallFlying => Some(Pose::FallFlying),
    }
}

/// What the render thread needs each frame, copied out under a short lock.
/// `LivingEntity.isDeadOrDying` as the camera sees it: `Some(deathTime)` while the health is
/// <= 0 (0 on the tick it got there), `None` alive.
pub fn death_time(dead: bool, death_ticks: u32) -> Option<u32> {
    dead.then_some(death_ticks)
}

#[derive(Clone, Debug)]
pub struct ViewState {
    /// Player eye position at the start of the last tick (`LocalPlayer::camera(0.0)`).
    pub prev_eye: DVec3,
    /// Player eye position after the last tick (`LocalPlayer::camera(1.0)`).
    pub eye: DVec3,
    /// Server rotation offset to add to the input layer's look (`LocalPlayer::look_offset`).
    pub look_offset: (f32, f32),
    /// When the last tick finished (partial tick is measured from here).
    pub tick_at: Instant,
    /// Game ticks since join (animated atlas textures play at this tick).
    pub animation_tick: u64,
    /// Whether the server has placed the player yet.
    pub has_position: bool,
    /// Player feet position after the last tick (`PlayerState::pos`).
    pub feet: DVec3,
    /// Eye height for the current pose (the camera sits this far above the feet).
    pub eye_height: f64,
    /// Time of day in ticks (first world clock of `set_time`; 6000 = noon until one arrives).
    pub time_of_day: i64,
    /// World age in ticks (`set_time`'s `world_age`), for game-time-keyed animations
    /// (vanilla `Level.getGameTime`: `BannerRenderer`'s wave phase).
    pub world_age: i64,
    /// Whether the current dimension has a sky.
    pub has_sky: bool,
    /// Server view distance in chunks.
    pub view_distance: i32,
    /// Block under the crosshair and the face hit (`LocalPlayer::target`).
    pub block_target: Option<(BlockPos, Direction)>,
    /// Whether the player's eyes are in water (`PlayerState::eyes_in_water`), for the HUD's
    /// air bubbles and the water overlay.
    pub eye_in_water: bool,
    /// Whether the camera is under the lava surface at the eye's block (vanilla's
    /// `Camera.getFluidInCamera` lava rule), for the submerged FOV.
    pub eye_in_lava: bool,
    /// Blindness effect remaining ticks (`None` without it), for the fog environment.
    pub blindness: Option<i32>,
    /// `LocalPlayer.getWaterVision()` (0 out of water), for the underwater fog distance.
    pub water_vision: f32,
    /// Whether we are a spectator (`Entity.isSpectator`), for the lava/powder-snow fog.
    pub spectator: bool,
    /// Whether we have fire resistance, for the lava fog distance.
    pub fire_resistance: bool,
    /// `BossHealthOverlay.shouldCreateWorldFog()`, for the thick boss fog.
    pub boss_world_fog: bool,
    /// Selected hotbar slot (`LocalPlayer::held_slot`), for the scroll wheel.
    pub held_slot: i32,
    /// What the first-person view holds this tick (`None` draws no hand: spectator, not yet
    /// placed). The skin it is drawn with is [`Shared::skin`], set once at join.
    pub first_person: Option<FirstPersonView>,
    /// The smoothed FOV modifier, last tick's value and this tick's (`Session::fov_modifier`
    /// / `old_fov_modifier`), interpolated per frame.
    pub fov_modifier: f32,
    pub fov_modifier_old: f32,
    /// Walk distance bracketing the last tick (`Session::walk_dist` / `walk_dist_o`).
    pub walk_dist: f32,
    pub walk_dist_o: f32,
    /// The bob amount bracketing the last tick (`Session::bob` / `bob_o`).
    pub bob: f32,
    pub bob_o: f32,
    /// Hurt ticks left and the damage direction in degrees (`Session::hurt_ticks` /
    /// `hurt_dir`).
    pub hurt_ticks: u32,
    pub hurt_dir: f32,
    /// Ticks since the local player died (`Session::death_ticks`, vanilla `deathTime`).
    pub death_ticks: u32,
    /// `LivingEntity.isDeadOrDying`: the player's health is <= 0 (true from the tick it hits 0,
    /// while `death_ticks` is still 0).
    pub dead: bool,
    /// Look angles (yaw, pitch) before and after the last tick, for the player's own model in
    /// third person.
    pub prev_look: (f32, f32),
    pub look: (f32, f32),
    /// Feet positions bracketing the last tick (like `prev_eye`/`eye`, for the own model).
    pub prev_feet: DVec3,
    /// Our cape's trailing point (`ClientAvatarState`'s cloak), ticked with the feet.
    pub cloak: haste_entity::avatar::Cloak,
    /// Body pose metadata for the own model (`None`: the physics pose only knows standing,
    /// crouching and swimming).
    pub pose: Option<haste_core::entity::Pose>,
    /// This tick's screen effects (`screen_effects`): the block the camera is inside, the
    /// worn head item's overlay, on fire, freeze ticks, portal and nausea, the light at the
    /// eye and whether we are scoping a spyglass.
    pub screen: crate::screen_effects::ViewEffects,
    /// Lightmap per-tick state the render thread folds into the lightmap by partial tick:
    /// the block-light flicker, the night-vision effect duration (`None` without it), the
    /// darkness effect's blend state, the boss-overlay world darkening (this tick and the
    /// last), the lightning sky flash, and the player's tick count for the darkness pulse.
    pub lightmap_flicker: f32,
    pub lightmap_night_vision: Option<i32>,
    /// Darkness effect remaining ticks (`None` without it), for the fog environment.
    pub lightmap_darkness: Option<i32>,
    pub lightmap_darkness_blend: haste_render::lightmap::BlendState,
    pub lightmap_boss_darkening: (f32, f32),
    pub lightmap_lightning: bool,
    pub lightmap_tick_count: i32,
    /// The weather levels bracketing the tick (`Level.oRainLevel`/`rainLevel` and the
    /// thunder pair), lerped by partial tick for the sky, fog and weather columns.
    pub rain_level: (f32, f32),
    pub thunder_level: (f32, f32),
    /// `AtmosphericFogEnvironment.rainFogMultiplier`, eased per frame by the tick.
    pub rain_fog_multiplier: f32,
    /// `ClientLevel.seaLevel` (the spawn info's; biome precipitation's snow line).
    pub sea_level: i32,
    /// `ClientLevelData.getHorizonHeight` (the dark disc's threshold).
    pub horizon_height: f64,
}

impl ViewState {
    /// [`death_time`] of this tick's state, for the death FOV and roll.
    pub fn death(&self) -> Option<u32> {
        death_time(self.dead, self.death_ticks)
    }
}

/// The per-tick part of the first-person hand: the held item, the light at the eye and this
/// tick's hand animation (swing, equip progress, bob). Every animated value is the pair of the
/// value at the start and at the end of the tick; the renderer lerps them by partial tick, the
/// way vanilla's `FirstPersonHandsAndItems.extractRenderState` does. The skin lives in
/// [`Shared::skin`] because it never changes during a connection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FirstPersonView {
    /// The main-hand stack's item (`None` draws the bare arm).
    pub item: Option<RegistryIndex>,
    /// (sky light, block light) at the eye position, `FirstPerson::light`'s order.
    pub light: (u8, u8),
    /// Swing progress 0..1 over the swing's ticks (0 when the hand does not swing).
    pub attack: (f32, f32),
    /// Equip progress: 1 held, 0 while the item switches.
    pub equip: (f32, f32),
    /// The hand's pitch lag (`LocalPlayer.xBob`), in degrees.
    pub x_bob: (f32, f32),
    /// The hand's yaw lag (`LocalPlayer.yBob`), in degrees.
    pub y_bob: (f32, f32),
}

impl FirstPersonView {
    /// The first-person hand for `item` in `world` at `eye`: the item held and the light at the
    /// block containing the eye (`EntityRenderer.getPackedLightCoords` probes
    /// `Entity.getLightProbePosition`, the eye position); block light is 15 while the player is
    /// on fire (`EntityRenderer.getBlockLightLevel`).
    pub fn sample(
        world: &dyn BlockView,
        eye: DVec3,
        item: Option<RegistryIndex>,
        on_fire: bool,
    ) -> Self {
        let p = BlockPos::new(
            eye.x.floor() as i32,
            eye.y.floor() as i32,
            eye.z.floor() as i32,
        );
        let block_light = if on_fire { 15 } else { world.block_light(p) };
        FirstPersonView {
            item,
            light: (world.sky_light(p), block_light),
            attack: (0.0, 0.0),
            equip: (1.0, 1.0),
            x_bob: (0.0, 0.0),
            y_bob: (0.0, 0.0),
        }
    }

    /// This tick's hand animation from the player's [`crate::hand::Hand`].
    pub fn with_hand(&mut self, hand: &crate::hand::Hand) {
        self.item = hand.shown_item();
        self.attack = hand.attack();
        self.equip = hand.equip();
        self.x_bob = hand.x_bob();
        self.y_bob = hand.y_bob();
    }

    /// The renderer's first-person hand for this view drawn with `skin` and, when `sleeve`,
    /// its right sleeve: two equal views with the same skin produce equal hands, so the
    /// caller's "changed?" check compares the view and the skin.
    pub fn with_skin(
        &self,
        skin: &haste_entity::skin::PlayerSkin,
        sleeve: bool,
    ) -> haste_render::FirstPerson {
        haste_render::FirstPerson {
            skin: Some(skin.body.clone()),
            slim: skin.model.is_slim(),
            sleeve,
            item: self.item,
            light: self.light,
            attack: pair(self.attack),
            equip: pair(self.equip),
            x_bob: pair(self.x_bob),
            y_bob: pair(self.y_bob),
        }
    }
}

/// A per-tick pair into the renderer's [`haste_render::TickPair`].
fn pair(v: (f32, f32)) -> haste_render::TickPair {
    haste_render::TickPair {
        start: v.0,
        end: v.1,
    }
}

/// The first-person hand's tick, in vanilla's order (`LivingEntity.baseTick`, then
/// `Player.tick`, then `FirstPersonHandsAndItems.tick`, with the walk bob in `aiStep` before
/// them); the interaction step above ran the attacks and uses that started the swings. In
/// spectator the hand is not drawn at all (vanilla hides it there), so it does not advance.
pub(super) fn tick_hand(
    driver: &mut crate::net::NetDriver,
    shared: &super::shared::Shared,
    tick_moved: f32,
) -> Option<FirstPersonView> {
    let first_person = if driver.session.player.game_mode == GameMode::Spectator {
        None
    } else {
        let on_fire = driver.session.own_flags().on_fire || driver.session.screen_probe.in_fire;
        let p = &mut driver.session.player;
        let attack_speed = p.held_attack_speed();
        p.hand.tick(
            tick_moved,
            (p.state.yaw, p.state.pitch),
            p.state.on_ground,
            matches!(p.state.pose, haste_physics::Pose::Swimming),
            p.held_stack(),
            attack_speed,
        );
        let eye = p.camera(1.0).0;
        let hand = &p.hand;
        let mut fp =
            FirstPersonView::sample(&*shared.world.read(), eye, hand.shown_item(), on_fire);
        fp.with_hand(hand);
        Some(fp)
    };
    first_person
}

/// The view update, the tick's last step (vanilla's per-tick render state extraction):
/// everything the render thread needs per frame is copied into [`ViewState`] here, bracketed
/// around the tick. Returns the player's feet position, for the block entity extraction
/// that follows.
pub(super) fn publish(
    driver: &crate::net::NetDriver,
    shared: &super::shared::Shared,
    first_person: Option<FirstPersonView>,
    prev: (DVec3, f32, f32),
    cur: (DVec3, f32, f32),
) -> DVec3 {
    let (prev_eye, prev_yaw, prev_pitch) = prev;
    let (eye, yaw, pitch) = cur;
    let s = &driver.session;
    let mut view = shared.view.lock();
    // The feet bracket the tick like the eye does; the eye height can change within it
    // (sneaking), so the old feet are the last view's, not the eye minus its height.
    let feet = s.player.state.pos;
    view.prev_feet = if view.has_position { view.feet } else { feet };
    let prev_feet = view.prev_feet;
    view.cloak.tick(prev_feet);
    view.prev_eye = prev_eye;
    view.eye = eye;
    view.prev_look = (prev_yaw, prev_pitch);
    view.look = (yaw, pitch);
    view.look_offset = s.player.look_offset();
    view.tick_at = Instant::now();
    view.animation_tick = view.animation_tick.wrapping_add(1);
    view.has_position = s.player.has_position;
    view.feet = feet;
    view.pose = player_pose(s);
    let lm = &s.lightmap;
    view.lightmap_flicker = lm.flicker;
    view.lightmap_night_vision = lm.night_vision;
    view.lightmap_darkness = lm.darkness;
    view.lightmap_darkness_blend = lm.darkness_blend;
    view.lightmap_boss_darkening = (lm.boss_darkening_o, lm.boss_darkening);
    view.lightmap_lightning = lm.sky_flash_time > 0;
    view.lightmap_tick_count = lm.tick_count;
    // The weather levels bracketing this tick (the render thread lerps by partial tick)
    // and the rain fog multiplier (`AtmosphericFogEnvironment.updateRainFogState` eased
    // by `deltaTicks * 0.2` toward rain level x sky-light factor x biome precipitation).
    {
        let w = &s.weather;
        view.rain_level = (w.rain_level_o, w.rain_level);
        view.thunder_level = (w.thunder_level_o, w.thunder_level);
        let target = if crate::net::WeatherState::can_have_weather(&s.dimension) {
            let world = shared.world.read();
            let precipitation = if biome_has_precipitation(&world, &shared.biome_climates, eye) {
                1.0
            } else {
                0.5
            };
            w.rain_level(1.0) * rain_fog_sky_light(&world, eye) * precipitation
        } else {
            0.0
        };
        view.rain_fog_multiplier += (target - view.rain_fog_multiplier) * 0.2;
        view.sea_level = s.sea_level;
        view.horizon_height =
            haste_render::sky::dark_disc::horizon_height(s.is_flat, s.dimension.min_y);
    }
    view.eye_height = s.player.eye_height();
    view.time_of_day = s.time_of_day.unwrap_or(6000);
    view.world_age = s.world_age;
    view.has_sky = s.dimension.has_skylight;
    view.block_target = s.player.target();
    view.eye_in_water = s.player.state.eyes_in_water;
    view.eye_in_lava = eye_in_lava(&shared.world.read(), &shared.data, eye);
    // The fog environment's player state (vanilla `FogRenderer.setupFog` reads these off the
    // entity): blindness and fire resistance from the active effects, spectator from the mode,
    // the water-vision ramp, and the boss overlay's world fog.
    let eff = |id: Option<i32>| {
        id.and_then(|i| s.player.int.effects.get(&i))
            .map(|e| e.duration)
    };
    view.blindness = eff(s.player.int.lightmap_effect_ids.blindness);
    view.fire_resistance = eff(s.player.int.lightmap_effect_ids.fire_resistance).is_some();
    view.spectator = s.player.game_mode == GameMode::Spectator;
    view.water_vision = s.player.get_water_vision();
    view.boss_world_fog = s.lightmap.boss_world_fog;
    view.held_slot = s.player.held_slot;
    view.first_person = first_person;
    view.fov_modifier = s.fov_modifier;
    view.fov_modifier_old = s.old_fov_modifier;
    // The view bobbing's walk distance and bob are the hand's own (vanilla's one
    // `ClientAvatarState` ticks both).
    let (walk_dist_o, walk_dist) = s.player.hand.walk_dist();
    view.walk_dist = walk_dist;
    view.walk_dist_o = walk_dist_o;
    let (bob_o, bob) = s.player.hand.bob();
    view.bob = bob;
    view.bob_o = bob_o;
    view.hurt_ticks = s.hurt_ticks;
    view.hurt_dir = s.hurt_dir;
    view.death_ticks = s.death_ticks;
    view.dead = s.player.health <= 0.0;
    // The screen effects, from this tick's world probe (the block the camera is inside, the
    // eye's light), the own entity's flags and freeze ticks, the worn head item and the active
    // effects. The eyes being in water comes from the physics' fluid at the eye position.
    let head_item = s.head_item().and_then(|i| {
        shared
            .data
            .registries
            .name("minecraft:item", i)
            .cloned()
            .and_then(|name| crate::screen_effects::camera_overlay(&name))
    });
    let screen = s.screen_effects();
    view.screen = crate::screen_effects::ViewEffects {
        block_overlay: s.screen_probe.block_overlay.clone(),
        eye_in_water: view.eye_in_water,
        eye_light: s.screen_probe.eye_light,
        // The client predicts its own fire from the fire block it overlaps
        // (`BaseFireBlock.fireIgnite`); the server never sends the owner their own
        // `set_entity_data`, so `own_flags.on_fire` stays false and the probe carries it.
        on_fire: s.own_flags().on_fire || s.screen_probe.in_fire,
        camera_overlay: head_item,
        frozen_ticks: s.own_flags().frozen_ticks,
        scoping: s.player.scoping(&shared.data),
        portal: (screen.prev_portal(), screen.portal()),
        nausea: s.player.int.nausea_blend_pair(),
        spinning_time: screen.spinning_time(),
        spinning_speed: screen.spinning_speed(),
    };
    drop(view);
    feet
}
