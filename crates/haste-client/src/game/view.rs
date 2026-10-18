//! What the render thread needs each frame of the local player and its hand (the per-tick
//! "view state" copied out under a short lock): the own model instance for third person, the
//! first-person hand's per-tick animation state, and the rain-fog helpers the tick's view
//! update reads.

use std::time::Instant;

use glam::DVec3;
use haste_core::entity::Pose;
use haste_core::math::Vec3;
use haste_core::model::{BodyTransform, EntityInstance, PoseOverrides};
use haste_core::view::BlockView;
use haste_core::{BlockPos, Direction, EntityId, RegistryIndex};
use haste_world::World;

use super::{interpolate, skins};

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

/// `AtmosphericFogEnvironment.updateRainFogState`'s sky-light factor: the sky light at the
/// camera's block, clamped `(light - 8) / 7`.
pub(super) fn rain_fog_sky_light(world: &World, eye: DVec3) -> f32 {
    use haste_core::view::BlockView as _;
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    ((world.sky_light(p) as f32) - 8.0) / 7.0
}

/// Whether the biome at the eye has precipitation (`updateRainFogState` halves the rain
/// fog when it does not).
pub(super) fn biome_has_precipitation(
    world: &World,
    climates: &[haste_render::BiomeClimate],
    eye: DVec3,
) -> bool {
    use haste_render::BiomeSource as _;
    let p = BlockPos::new(
        eye.x.floor() as i32,
        eye.y.floor() as i32,
        eye.z.floor() as i32,
    );
    let biome = crate::app::biomes::WorldBiomes(world).biome(p).0 as usize;
    climates.get(biome).is_some_and(|c| c.has_precipitation)
}

/// The local player's own render instance for third person (the server does not spawn us as an
/// entity, so it is not in the store): the humanoid model at the interpolated feet, body yaw
/// and head pitched to the interpolated look, and its pose's adjustments like any player. The
/// limb swing the store drives for other entities is omitted (no per-tick walk phase for the
/// local player yet).
pub fn player_instance(
    id: EntityId,
    view: &ViewState,
    partial: f32,
    skin: &haste_entity::skin::PlayerSkin,
) -> EntityInstance {
    let pos = interpolate(view.prev_feet, view.feet, partial);
    let yaw = view.prev_look.0 + (view.look.0 - view.prev_look.0) * partial;
    let pitch = view.prev_look.1 + (view.look.1 - view.prev_look.1) * partial;
    let mut pose = PoseOverrides::default();
    use haste_core::parts;
    match view.pose {
        Some(Pose::Crouching) => {
            pose.set_rotation(parts::BODY, Vec3::new(0.5, 0.0, 0.0));
            pose.set_rotation(parts::HEAD, Vec3::new(-0.5, 0.0, 0.0));
        }
        Some(Pose::Swimming | Pose::FallFlying | Pose::SpinAttack) => {
            pose.set_rotation(
                parts::BODY,
                Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0),
            );
            pose.set_rotation(
                parts::RIGHT_ARM,
                Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, 0.4),
            );
            pose.set_rotation(
                parts::LEFT_ARM,
                Vec3::new(-std::f32::consts::FRAC_PI_2, 0.0, -0.4),
            );
        }
        _ => {
            // Head aim: the head follows the look's pitch (the body and head yaw are the same
            // here, so no yaw component).
            pose.set_rotation(parts::HEAD, Vec3::new(pitch.to_radians(), 0.0, 0.0));
        }
    }
    let mut inst = EntityInstance {
        id,
        model: haste_entity::models::humanoid(false),
        pos,
        yaw,
        pitch,
        head_yaw: yaw,
        scale: 1.0,
        pose,
        body: BodyTransform::default(),
        body_scale: Vec3::ONE,
        skin: None,
        nametag: None,
        below_name: None,
        // `AvatarRenderer` passes shadowRadius 0.5 (LivingEntityRenderer, player scale 1).
        shadow_radius: 0.5,
        shadow_strength: 1.0,
        invisible: false,
        is_discrete: view.pose == Some(Pose::Crouching),
        // `EntityAttachment.NAME_TAG` AT_HEIGHT fallback = the player's 1.8 bb height.
        name_tag_y: 1.8,
        // Our own model flashes red in third person like other entities: full strength on the
        // tick of the hit, fading over the 10 hurt ticks (vanilla's hurtTime / hurtDuration).
        hurt: (view.hurt_ticks.min(crate::net::HURT_TILT_TICKS) as f32
            / crate::net::HURT_TILT_TICKS as f32),
        overlays: Default::default(),
        item: None,
        decoration: None,
        hands: [None, None],
        equipment: Vec::new(),
        head_item: None,
    };
    skins::dress_local(&mut inst, skin, view, partial);
    inst
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
