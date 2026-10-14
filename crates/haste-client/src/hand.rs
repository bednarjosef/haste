//! The first-person hand's per-tick state: the swing, the equip (item switch) progress and the
//! walk bob. The client owns all of it and the renderer only lerps the two ends of each tick,
//! the way vanilla's render state extraction does (`FirstPersonHandsAndItems.extractRenderState`).
//!
//! Only the main hand is modelled (the off hand's item, its swing and its equip progress are a
//! later task), so the `invert` factor of vanilla's arm transforms (`mainArm == RIGHT ? 1 : -1`)
//! is the right-hand one everywhere.
//!
//! Use animations (eat, drink, bow, shield, brush, spear, crossbow) are a later task as well:
//! `FirstPersonHandsAndItemsRenderer.submitArmWithItem`'s use branches need, per hand,
//! `isUsingItem`, `useItemRemainingTicks` and `useDuration` (`getUseDuration`, from the held
//! item's `use_duration` component) plus the item's `use_animation` component, and the client
//! has none of them yet: `interact_step` only sends the packets and tracks no "using item"
//! state, so `release_use_item` is never sent and the use ticks are not counted. The hand state
//! here has room for them (`Hand::item_used` is the hook the use dip goes through).
//!
//! A few accessors are for the tests and the later use-animation work (the swing's lerped
//! progress, `isSwinging`), so they are unused for now.
#![allow(dead_code)]

use haste_core::RegistryIndex;

/// Ticks a swing takes (`SwingAnimation.DEFAULT`'s duration; the item's own
/// `attack_animation`/`interact_animation` component is not decoded yet).
pub const SWING_DURATION: u32 = 6;
/// How far the equip progress moves per tick (`FirstPersonHandsAndItems`'s 0.4).
const HEIGHT_STEP: f32 = 0.4;
/// Equip progress below which the new item is shown (`FirstPersonHandsAndItems`'s 0.1).
const HEIGHT_SWAP: f32 = 0.1;
/// `Attributes.ATTACK_SPEED`'s default; the equip progress returns at the server's value.
pub const DEFAULT_ATTACK_SPEED: f64 = 4.0;
/// `LocalPlayer.move`: the walked distance added to `ClientAvatarState.walkDist`.
const WALK_DISTANCE_SCALE: f32 = 0.6;

/// `Mth.clamp`.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `Mth.lerp`.
pub fn lerp(delta: f32, start: f32, end: f32) -> f32 {
    start + delta * (end - start)
}

/// `LivingEntity.SwingState`: which hand swings, for how long, and how far (0..1) the swing has
/// got. Ticked once per game tick and lerped per frame by the renderer.
#[derive(Clone, Copy, Debug)]
pub struct Swing {
    /// The swinging hand (the main hand unless a later task swings the off hand).
    main_hand: bool,
    /// `SwingState.currentSwing != null`.
    swinging: bool,
    /// `SwingDescription.durationTicks`.
    duration: u32,
    /// `SwingState.ticks` since the swing started.
    ticks: u32,
    /// `SwingState.animation`, the progress at the end of the last tick.
    animation: f32,
    /// `SwingState.oldAnimation`, at the end of the tick before.
    prev_animation: f32,
}

impl Default for Swing {
    fn default() -> Self {
        Swing {
            main_hand: true,
            swinging: false,
            duration: SWING_DURATION,
            ticks: 0,
            animation: 0.0,
            prev_animation: 0.0,
        }
    }
}

impl Swing {
    /// `LivingEntity.swing` -> `SwingState.startIfAble`: a swing that is less than half way
    /// through is not restarted (`Minecraft.startAttack`/`continueAttack`/`startUseItem` call
    /// this on every attack, block hit and successful item use).
    pub fn start(&mut self) {
        self.start_hand(true);
    }

    fn start_hand(&mut self, main_hand: bool) {
        if self.swinging && self.ticks > 0 && self.ticks <= self.duration / 2 {
            return;
        }
        // `SwingState.start` resets the progress only when the swing description changed
        // (restarting the very same swing keeps `oldAnimation`/`animation` where they are).
        if !self.swinging || self.main_hand != main_hand {
            self.prev_animation = 0.0;
            self.animation = 0.0;
        }
        self.main_hand = main_hand;
        self.swinging = true;
        self.duration = SWING_DURATION;
        self.ticks = 0;
    }

    /// `LivingEntity.baseTick` -> `SwingState.tick`, once per game tick.
    pub fn tick(&mut self) {
        self.prev_animation = self.animation;
        if self.swinging {
            self.animation = (self.ticks as f32 / self.duration as f32).min(1.0);
            if self.ticks > self.duration {
                self.swinging = false;
                self.animation = 0.0;
            } else {
                self.ticks += 1;
            }
        } else {
            self.animation = 0.0;
        }
    }

    /// `LivingEntity.getSwingAnimation` -> `SwingState.getAnimation`, lerped by the renderer.
    pub fn animation(&self, partial: f32) -> f32 {
        if self.animation >= self.prev_animation {
            lerp(partial, self.prev_animation, self.animation)
        } else {
            lerp(partial, self.prev_animation, self.animation + 1.0)
        }
    }

    /// The swing progress at the ends of the last two ticks, 0 when the swing is not in the hand
    /// the renderer draws (`FirstPersonHandsAndItems.extractRenderState`'s `mainHandAttack`).
    pub fn pair(&self) -> (f32, f32) {
        if self.main_hand {
            (self.prev_animation, self.animation)
        } else {
            (0.0, 0.0)
        }
    }

    /// Whether the main hand is swinging (`LivingEntity.isSwinging`).
    pub fn is_swinging(&self) -> bool {
        self.swinging
    }
}

/// `FirstPersonHandsAndItems`'s main hand: the item being shown and how far it is down
/// (0 while it switches, 1 when it is up), plus `Player.itemSwapTicker`.
#[derive(Clone, Debug)]
pub struct Equip {
    /// The item type currently shown; it only changes once the hand has dipped.
    item: Option<RegistryIndex>,
    height: f32,
    prev_height: f32,
    /// `Player.itemSwapTicker`, reset when the held item's type changes.
    swap_ticker: u32,
    /// `Player.attackStrengthTicker`, reset on an attack and on an item type change
    /// (`Player.getAttackStrengthScale`); the HUD attack indicator reads it.
    attack_strength_ticker: u32,
    /// The held item's `minecraft:attack_speed` this tick (for the scale's delay).
    attack_speed: f64,
    /// `Player.lastItemInMainHand`, the type seen last tick.
    last_item: Option<RegistryIndex>,
}

impl Default for Equip {
    fn default() -> Self {
        // Vanilla's `FirstPersonHandsAndItems` fields start at their Java defaults (0.0), with
        // an empty shown item: the hand is raised in over the first ticks after joining.
        Equip {
            item: None,
            height: 0.0,
            prev_height: 0.0,
            swap_ticker: 0,
            attack_strength_ticker: 0,
            attack_speed: DEFAULT_ATTACK_SPEED,
            last_item: None,
        }
    }
}

impl Equip {
    /// `Player.tick`'s tickers and item-type check, then `FirstPersonHandsAndItems.tick`.
    /// `item` is this tick's main-hand item type and `attack_speed` the player's
    /// `minecraft:attack_speed` attribute.
    pub fn tick(&mut self, item: Option<RegistryIndex>, attack_speed: f64) {
        self.prev_height = self.height;
        self.attack_speed = attack_speed;
        // `Player.tick`: both tickers advance, and a different item type resets both.
        self.swap_ticker += 1;
        self.attack_strength_ticker += 1;
        if self.last_item != item {
            self.swap_ticker = 0;
            self.attack_strength_ticker = 0;
        }
        self.last_item = item;
        // `shouldInstantlyReplaceVisibleItem`: the same item type is shown at once; a different
        // one plays the swap animation (every vanilla item has `hand_animation_on_swap`).
        if self.item != item && !SWAP_ANIMATION {
            self.item = item;
        }
        // `Player.getItemSwapScale(1.0)`: the equip progress returns over the attack-speed delay.
        let attack_anim = clamp(
            (self.swap_ticker as f32 + 1.0) / swap_delay(attack_speed),
            0.0,
            1.0,
        );
        let target = if self.item != item {
            0.0
        } else {
            attack_anim * attack_anim * attack_anim
        };
        self.height += clamp(target - self.height, -HEIGHT_STEP, HEIGHT_STEP);
        self.height = clamp(self.height, 0.0, 1.0);
        if self.height < HEIGHT_SWAP {
            self.item = item;
        }
    }

    /// The equip progress at the ends of the last two ticks (1 held, 0 while switching).
    pub fn pair(&self) -> (f32, f32) {
        (self.prev_height, self.height)
    }

    /// `FirstPersonHandsAndItems.itemUsed`: the hand drops at once when the held item was used
    /// (`Minecraft.startUseItem`), which then switches the item below 0.1 and raises it again.
    pub fn item_used(&mut self) {
        self.height = 0.0;
    }

    /// The item type the hand shows (the renderer draws the arm while it is `None`).
    pub fn item(&self) -> Option<RegistryIndex> {
        self.item
    }

    /// `Player.getAttackStrengthScale(a)`: the attack charge 0..1 over the held item's delay.
    pub fn attack_strength_scale(&self, a: f32) -> f32 {
        clamp(
            (self.attack_strength_ticker as f32 + a) / swap_delay(self.attack_speed),
            0.0,
            1.0,
        )
    }

    /// `Player.resetAttackStrengthTicker`: both tickers (`MultiPlayerGameMode.stopDestroyBlock`).
    pub fn reset_attack_strength_ticker(&mut self) {
        self.attack_strength_ticker = 0;
        self.swap_ticker = 0;
    }

    /// `Player.resetOnlyAttackStrengthTicker` (`Player.onAttack`).
    pub fn reset_only_attack_strength_ticker(&mut self) {
        self.attack_strength_ticker = 0;
    }
}

/// Whether a newly held item plays the swap animation (`ClientItem.Properties`
/// `hand_animation_on_swap`; every vanilla item does, so the dip always plays).
const SWAP_ANIMATION: bool = true;

/// `Player.getCurrentItemAttackStrengthDelay`: 20 ticks divided by the attack speed.
fn swap_delay(attack_speed: f64) -> f32 {
    let speed = if attack_speed > 0.0 {
        attack_speed
    } else {
        DEFAULT_ATTACK_SPEED
    };
    (20.0 / speed) as f32
}

/// `ClientAvatarState`'s bob plus `LocalPlayer.xBob`/`yBob`: the walk bob's phase and amplitude,
/// and the hand's lag behind the camera turn.
#[derive(Clone, Copy, Debug)]
pub struct Bob {
    /// `ClientAvatarState.walkDist`, in blocks walked.
    walk_dist: f32,
    prev_walk_dist: f32,
    /// `ClientAvatarState.bob`, the amplitude (0..0.1).
    bob: f32,
    prev_bob: f32,
    /// `LocalPlayer.xBob`: pitch lag, in degrees.
    x_bob: f32,
    prev_x_bob: f32,
    /// `LocalPlayer.yBob`: yaw lag, in degrees.
    y_bob: f32,
    prev_y_bob: f32,
    /// The previous tick's on-ground and horizontal speed: `AbstractClientPlayer.updateBob`
    /// runs before this tick's travel, so it reads the previous tick's delta movement.
    was_on_ground: bool,
    speed: f32,
}

impl Default for Bob {
    fn default() -> Self {
        Bob {
            walk_dist: 0.0,
            prev_walk_dist: 0.0,
            bob: 0.0,
            prev_bob: 0.0,
            x_bob: 0.0,
            prev_x_bob: 0.0,
            y_bob: 0.0,
            prev_y_bob: 0.0,
            was_on_ground: true,
            speed: 0.0,
        }
    }
}

impl Bob {
    /// One tick, in vanilla's order: `AbstractClientPlayer.updateBob` (from the previous tick's
    /// delta movement), `ClientAvatarState.tick` + `LocalPlayer.move`'s walked distance, then
    /// `LocalPlayer.applyInput`'s bob lag. `moved` is this tick's horizontal travel in blocks.
    pub fn tick(&mut self, moved: f32, look: (f32, f32), on_ground: bool, swimming: bool) {
        let target_bob = if self.was_on_ground && !swimming {
            self.speed.min(0.1)
        } else {
            0.0
        };
        self.prev_bob = self.bob;
        self.bob += (target_bob - self.bob) * 0.4;
        self.was_on_ground = on_ground;
        self.speed = moved;
        self.prev_walk_dist = self.walk_dist;
        self.walk_dist += moved * WALK_DISTANCE_SCALE;
        self.prev_x_bob = self.x_bob;
        self.prev_y_bob = self.y_bob;
        self.x_bob += (look.1 - self.x_bob) * 0.5;
        self.y_bob += (look.0 - self.y_bob) * 0.5;
    }

    /// The bob amplitude at the ends of the last two ticks.
    pub fn bob(&self) -> (f32, f32) {
        (self.prev_bob, self.bob)
    }

    /// The walked distance at the ends of the last two ticks.
    pub fn walk_dist(&self) -> (f32, f32) {
        (self.prev_walk_dist, self.walk_dist)
    }

    /// The pitch lag (`xBob`) at the ends of the last two ticks, in degrees.
    pub fn x_bob(&self) -> (f32, f32) {
        (self.prev_x_bob, self.x_bob)
    }

    /// The yaw lag (`yBob`) at the ends of the last two ticks, in degrees.
    pub fn y_bob(&self) -> (f32, f32) {
        (self.prev_y_bob, self.y_bob)
    }

    /// `ClientAvatarState.getBackwardsInterpolatedWalkDistance`: the bob's phase is the walked
    /// distance, negated and interpolated from the ends of the tick.
    pub fn backwards_walk_dist(&self, partial: f32) -> f32 {
        -(lerp(partial, self.prev_walk_dist, self.walk_dist))
    }
}

/// The local player's first-person hand: everything the renderer needs each frame, as the two
/// ends of the last game tick. Vanilla's `LivingEntity.SwingState`,
/// `FirstPersonHandsAndItems` and `ClientAvatarState` (plus `LocalPlayer`'s bob fields), ticked
/// once per game tick in the same order.
#[derive(Clone, Debug, Default)]
pub struct Hand {
    swing: Swing,
    equip: Equip,
    bob: Bob,
}

impl Hand {
    /// A hand holding nothing, with the equip progress up (as after the first tick).
    pub fn new() -> Hand {
        Hand::default()
    }

    /// Starts a swing of the main hand (vanilla `Minecraft.startAttack`/`continueAttack`/
    /// `startUseItem` all call `LivingEntity.swing`).
    pub fn swing(&mut self) {
        self.swing.start();
    }

    /// One 20 Hz tick. `moved` is the horizontal distance travelled this tick, `look` the
    /// player's rotation (yaw, pitch) in degrees at its end, `item` the main-hand item type and
    /// `attack_speed` the player's `minecraft:attack_speed` attribute.
    pub fn tick(
        &mut self,
        moved: f32,
        look: (f32, f32),
        on_ground: bool,
        swimming: bool,
        item: Option<RegistryIndex>,
        attack_speed: f64,
    ) {
        // `LivingEntity.baseTick` runs before `Player.tick` and both run before
        // `FirstPersonHandsAndItems.tick`, which the walk bob precedes (`LocalPlayer.aiStep`).
        self.swing.tick();
        self.equip.tick(item, attack_speed);
        self.bob.tick(moved, look, on_ground, swimming);
    }

    /// The item type the hand shows: the new one once the equip progress has dipped below 0.1
    /// (`FirstPersonHandsAndItems.mainHandItem`), so the renderer draws the old item while the
    /// hand is down and the bare arm once it is gone.
    pub fn shown_item(&self) -> Option<RegistryIndex> {
        self.equip.item()
    }

    /// The swing progress at the ends of the last tick (0 when not swinging).
    pub fn attack(&self) -> (f32, f32) {
        self.swing.pair()
    }

    /// The equip progress at the ends of the last tick (1 held, 0 while switching).
    pub fn equip(&self) -> (f32, f32) {
        self.equip.pair()
    }

    /// The hand dropped by a used item (`FirstPersonHandsAndItems.itemUsed`).
    pub fn item_used(&mut self) {
        self.equip.item_used();
    }

    /// The walk bob amplitude at the ends of the last tick.
    pub fn bob(&self) -> (f32, f32) {
        self.bob.bob()
    }

    /// The walked distance at the ends of the last tick.
    pub fn walk_dist(&self) -> (f32, f32) {
        self.bob.walk_dist()
    }

    /// The pitch lag (`xBob`) at the ends of the last tick, in degrees.
    pub fn x_bob(&self) -> (f32, f32) {
        self.bob.x_bob()
    }

    /// The yaw lag (`yBob`) at the ends of the last tick, in degrees.
    pub fn y_bob(&self) -> (f32, f32) {
        self.bob.y_bob()
    }

    /// The swing progress lerped by partial tick, for a test or the HUD.
    pub fn animation(&self, partial: f32) -> f32 {
        self.swing.animation(partial)
    }

    /// The attack charge 0..1 for the HUD attack indicator (`Player.getAttackStrengthScale`).
    pub fn attack_strength_scale(&self, a: f32) -> f32 {
        self.equip.attack_strength_scale(a)
    }

    /// `Player.resetAttackStrengthTicker` (abort breaking, item change).
    pub fn reset_attack_strength_ticker(&mut self) {
        self.equip.reset_attack_strength_ticker();
    }

    /// `Player.resetOnlyAttackStrengthTicker` (`Player.onAttack`).
    pub fn reset_only_attack_strength_ticker(&mut self) {
        self.equip.reset_only_attack_strength_ticker();
    }

    /// Whether the main hand is swinging.
    pub fn is_swinging(&self) -> bool {
        self.swing.is_swinging()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stone() -> Option<RegistryIndex> {
        Some(RegistryIndex(1))
    }

    /// One vanilla tick: no item held (the bare arm), standing on the ground, no swing.
    fn tick(h: &mut Hand, moved: f32) {
        h.tick(moved, (0.0, 0.0), true, false, None, 4.0);
    }

    /// Ticks until the equip progress is back up (vanilla raises the hand in over the first
    /// ticks after joining, since `FirstPersonHandsAndItems` starts at 0).
    fn settle(h: &mut Hand, item: Option<RegistryIndex>) {
        for _ in 0..12 {
            h.tick(0.0, (0.0, 0.0), true, false, item, 4.0);
        }
        assert_eq!(h.equip().1, 1.0);
    }

    #[test]
    fn swing_progress_runs_over_six_ticks() {
        // `LivingEntity.SwingState` with `SwingAnimation.DEFAULT` (duration 6): the swing starts
        // on the tick of the attack with progress 0, and `tick` then advances ticks/duration, so
        // the visible frames are the six ticks after it.
        let mut h = Hand::new();
        h.swing();
        let mut seen = Vec::new();
        for _ in 0..9 {
            tick(&mut h, 0.0);
            seen.push(h.attack());
            let (start, end) = *seen.last().unwrap();
            assert_eq!(start, h.swing.prev_animation, "start is the previous tick");
            assert_eq!(end, h.swing.animation, "end is this tick");
        }
        // progress = ticks / 6 at the end of each tick; the tick after the last clears it, and
        // vanilla's `getAnimation` lerps 1 -> 1 over that tick so nothing jumps.
        for (got, want) in seen.iter().zip([
            (0.0, 0.0),
            (0.0, 1.0 / 6.0),
            (1.0 / 6.0, 2.0 / 6.0),
            (2.0 / 6.0, 3.0 / 6.0),
            (3.0 / 6.0, 4.0 / 6.0),
            (4.0 / 6.0, 5.0 / 6.0),
            (5.0 / 6.0, 1.0),
            (1.0, 0.0),
            (0.0, 0.0),
        ]) {
            assert!(
                (got.0 - want.0).abs() < 1e-7 && (got.1 - want.1).abs() < 1e-7,
                "{got:?} != {want:?}"
            );
        }
        // Mid-swing the renderer lerps the pair: half way between the 0 and 1/6 ticks.
        let mut h = Hand::new();
        h.swing();
        tick(&mut h, 0.0);
        tick(&mut h, 0.0);
        let (start, end) = h.attack();
        assert!(start.abs() < 1e-7);
        assert!((end - 1.0 / 6.0).abs() < 1e-7);
        assert!((h.animation(0.5) - 1.0 / 12.0).abs() < 1e-7);
    }

    #[test]
    fn a_swing_less_than_half_way_is_not_restarted() {
        let mut h = Hand::new();
        h.swing();
        for _ in 0..3 {
            tick(&mut h, 0.0);
        }
        // Half way (ticks 3 of 6): `startIfAble` refuses the restart, the swing carries on.
        h.swing();
        assert_eq!(h.attack().1, 2.0 / 6.0);
        tick(&mut h, 0.0);
        assert_eq!(h.attack().1, 3.0 / 6.0);
        // Past the half way a new swing starts over from 0.
        h.swing();
        tick(&mut h, 0.0);
        assert_eq!(h.attack(), (3.0 / 6.0, 0.0));
    }

    #[test]
    fn no_swing_means_no_progress() {
        let mut h = Hand::new();
        for _ in 0..10 {
            tick(&mut h, 0.0);
        }
        assert_eq!(h.attack(), (0.0, 0.0));
        assert!(!h.is_swinging());
        // The bare arm (no item) is drawn.
        assert_eq!(h.shown_item(), None);
    }

    #[test]
    fn equip_dips_on_an_item_change_and_comes_back() {
        // `FirstPersonHandsAndItems.tick`: with the same item held the progress falls 0.4 per
        // tick while the shown item differs from the held one, the new item appears once it is
        // below 0.1, and it then rises towards `getItemSwapScale(1.0)^3` =
        // ((itemSwapTicker + 1) / (20 / attack_speed))^3 with attack_speed 4 (a 5 tick delay).
        let mut h = Hand::new();
        settle(&mut h, None);
        let mut seen = Vec::new();
        for _ in 0..6 {
            h.tick(0.0, (0.0, 0.0), true, false, stone(), 4.0);
            seen.push((h.equip(), h.shown_item()));
        }
        // 1.0 -> 0.6 -> 0.2 -> 0.0 (the item switches) -> 0.4 -> 0.8.
        for (got, want) in seen
            .iter()
            .map(|(p, _)| p.1)
            .zip([0.6, 0.2, 0.0, 0.4, 0.8, 1.0])
        {
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
        // The bare arm stays until the progress drops below 0.1.
        assert_eq!(seen[0].1, None);
        assert_eq!(seen[1].1, None);
        assert_eq!(seen[2].1, stone());
        // A slow attack speed brings the item back slowly: with attack_speed 1 the swap delay
        // is 20 ticks, so the first tick after the switch only reaches ((3 + 1) / 20)^3 = 0.008.
        let mut slow = Hand::new();
        settle(&mut slow, None);
        for _ in 0..4 {
            slow.tick(0.0, (0.0, 0.0), true, false, stone(), 1.0);
        }
        assert!(
            (slow.equip().1 - 0.2f32.powi(3)).abs() < 1e-6,
            "{}",
            slow.equip().1
        );
        assert_eq!(slow.shown_item(), stone());
    }

    #[test]
    fn the_same_item_never_dips() {
        let mut h = Hand::new();
        settle(&mut h, stone());
        for _ in 0..5 {
            h.tick(0.0, (0.0, 0.0), true, false, stone(), 4.0);
            assert_eq!(h.equip().1, 1.0, "the held item stays put");
        }
    }

    #[test]
    fn walk_bob_follows_the_walk_cycle() {
        // `ClientAvatarState` (walkDist = 0.6 * horizontal distance) and
        // `AbstractClientPlayer.updateBob` (the bob moves 0.4 per tick towards
        // min(0.1, the *previous* tick's horizontal speed)). Walking 0.1 blocks per tick.
        let mut h = Hand::new();
        let mut seen = Vec::new();
        for _ in 0..10 {
            tick(&mut h, 0.1);
            seen.push((h.walk_dist(), h.bob()));
        }
        // walkDist after n ticks of 0.1 blocks: 0.06 * n.
        for (i, (w, _)) in seen.iter().enumerate() {
            let n = (i + 1) as f32;
            assert!((w.1 - 0.06 * n).abs() < 1e-6, "walkDist {}", w.1);
            assert!(
                (w.0 - 0.06 * (n - 1.0)).abs() < 1e-6,
                "prev walkDist {}",
                w.0
            );
        }
        // The bob converges to the 0.1 cap by 0.4 per tick, one tick behind the walk.
        for (got, want) in seen.iter().map(|(_, b)| b.1).zip([
            0.0, 0.04, 0.064, 0.0784, 0.08704, 0.092224, 0.0953344, 0.09720064, 0.09832038,
            0.09899223,
        ]) {
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
        // Leaving the ground: the first airborne tick still reads the ground tick's speed, the
        // one after it multiplies the bob by 0.6.
        h.tick(0.0, (0.0, 0.0), false, false, None, 4.0);
        let in_air = h.bob().1;
        assert!((in_air - (0.09899223 + (0.1 - 0.09899223) * 0.4)).abs() < 1e-6);
        h.tick(0.0, (0.0, 0.0), false, false, None, 4.0);
        assert!((h.bob().1 - in_air * 0.6).abs() < 1e-6, "{}", h.bob().1);
    }

    #[test]
    fn the_bob_phase_is_the_walked_distance() {
        let mut h = Hand::new();
        for _ in 0..5 {
            tick(&mut h, 0.25);
        }
        // Backwards interpolated: -(walkDist + (walkDist - prevWalkDist) * partial).
        let (prev, end) = h.walk_dist();
        assert!((end - 0.75).abs() < 1e-6, "{end}");
        for partial in [0.0, 0.25, 0.5, 1.0] {
            let want = -(prev + (end - prev) * partial);
            assert!((h.bob.backwards_walk_dist(partial) - want).abs() < 1e-6);
        }
    }

    #[test]
    fn the_hand_lags_behind_the_look() {
        // `LocalPlayer.applyInput`: xBob/yBob move half of the remaining angle towards the
        // player's new rotation each tick (yaw 10, 20, ... 80 over eight ticks).
        let mut h = Hand::new();
        let mut yaw = 0.0;
        let (mut want_yaw, mut want_pitch) = (0.0, 0.0);
        for _ in 0..8 {
            yaw += 10.0;
            h.tick(0.0, (yaw, yaw / 2.0), true, false, None, 4.0);
            want_yaw += (yaw - want_yaw) * 0.5;
            want_pitch += (yaw / 2.0 - want_pitch) * 0.5;
        }
        // yBob ends at 70.0390625 and xBob at 35.01953125 (exact binary fractions).
        assert_eq!(h.y_bob().1, 70.039_06, "{}", h.y_bob().1);
        assert!(
            (h.y_bob().1 - want_yaw).abs() < 1e-4,
            "{} != {want_yaw}",
            h.y_bob().1
        );
        assert_eq!(h.x_bob().1, 35.019_53, "{}", h.x_bob().1);
        assert!((h.x_bob().1 - want_pitch).abs() < 1e-4);
        // The pair is the previous tick's value and this one's.
        assert!((h.y_bob().0 - 60.078_125).abs() < 1e-4, "{}", h.y_bob().0);
    }

    #[test]
    fn attack_strength_charges_over_the_item_delay_and_resets() {
        // `Player.attackStrengthTicker` advances each tick and `getAttackStrengthScale` is
        // (ticker + a) / (20 / attack_speed), clamped. The bare hand's attack speed 4 gives a
        // 5 tick delay; a diamond sword's 1.6 gives 12.5.
        let mut h = Hand::new();
        let mut scale = Vec::new();
        for _ in 0..6 {
            tick(&mut h, 0.0);
            scale.push(h.attack_strength_scale(0.0));
        }
        // None -> None on the first tick (no item type change), so the ticker runs 1..6.
        for (got, want) in scale.iter().zip([0.2, 0.4, 0.6, 0.8, 1.0, 1.0]) {
            assert!((got - want).abs() < 1e-6, "{got} != {want}");
        }
        // `Player.onAttack` resets only the attack ticker.
        h.reset_only_attack_strength_ticker();
        tick(&mut h, 0.0);
        assert!((h.attack_strength_scale(0.0) - 0.2).abs() < 1e-6);
        // An item type change resets it (`Player.tick`).
        h.tick(0.0, (0.0, 0.0), true, false, stone(), 1.6);
        assert_eq!(h.attack_strength_scale(0.0), 0.0);
        // The sword's slower attack speed: 12 ticks -> 12 / 12.5 = 0.96.
        for _ in 0..12 {
            h.tick(0.0, (0.0, 0.0), true, false, stone(), 1.6);
        }
        assert!((h.attack_strength_scale(0.0) - 0.96).abs() < 1e-6);
    }

    #[test]
    fn a_used_item_puts_the_hand_down() {
        // `FirstPersonHandsAndItems.itemUsed` sets the progress to 0 at once, so the tick that
        // follows it starts at 0 and rises back.
        let mut h = Hand::new();
        settle(&mut h, None);
        h.item_used();
        tick(&mut h, 0.0);
        assert_eq!(h.equip(), (0.0, 0.4));
        // With a real item held the shown item does not change (the type is the same).
        let mut h = Hand::new();
        settle(&mut h, stone());
        h.item_used();
        h.tick(0.0, (0.0, 0.0), true, false, stone(), 4.0);
        assert_eq!(h.equip(), (0.0, 0.4));
        assert_eq!(h.shown_item(), stone());
    }
}
