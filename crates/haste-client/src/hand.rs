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
//! item's `use_duration` component) plus the item's `use_animation` component. The interaction
//! step tracks the state now (`Interaction::is_using_item`/`using`, driven by the
//! `minecraft:consumable` component and `release_use_item`), but the hand here still draws no
//! use dip: `Hand::item_used` is the hook it would go through.
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
    /// Like `SwingState.getAnimation`, a falling end (the tick the swing ends, or a restart
    /// that keeps the progress) is wrapped past 1 (`animation + 1.0`), so the renderer's plain
    /// lerp holds the rest pose instead of retracing the swing backwards.
    pub fn pair(&self) -> (f32, f32) {
        if !self.main_hand {
            return (0.0, 0.0);
        }
        if self.animation < self.prev_animation {
            (self.prev_animation, self.animation + 1.0)
        } else {
            (self.prev_animation, self.animation)
        }
    }

    /// Whether the main hand is swinging (`LivingEntity.isSwinging`).
    pub fn is_swinging(&self) -> bool {
        self.swinging
    }
}

/// The stack in a hand as the equip sees it: the item type (for `Player.tick`'s tickers, which
/// compare types with `ItemStack.isSameItem`) and a swap key over the count and the components
/// that are not `DataComponentType.ignoreSwapAnimation` (`damage`, `max_damage`,
/// `max_stack_size`), standing in for `ItemStack.matchesIgnoringComponents` in
/// `FirstPersonHandsAndItems.shouldInstantlyReplaceVisibleItem`: two stacks with the same type
/// and key are shown at once, anything else plays the swap dip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeldStack {
    pub item: Option<RegistryIndex>,
    pub swap_key: u64,
}

impl HeldStack {
    /// A stack known by its type only (the tests, and callers without component data).
    pub fn of(item: Option<RegistryIndex>) -> HeldStack {
        HeldStack { item, swap_key: 0 }
    }
}

/// `FirstPersonHandsAndItems`'s main hand: the item being shown and how far it is down
/// (0 while it switches, 1 when it is up), plus `Player.itemSwapTicker`.
#[derive(Clone, Debug)]
pub struct Equip {
    /// The stack currently shown; it only changes once the hand has dipped.
    item: HeldStack,
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
            item: HeldStack::default(),
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
    /// `stack` is this tick's main-hand stack and `attack_speed` the player's
    /// `minecraft:attack_speed` attribute. `hands_busy` is vanilla `LocalPlayer.rideTick`'s
    /// `handsBusy` (a boat we steer with a movement key held): the busy branch of
    /// `FirstPersonHandsAndItems.tick` dips the height instead of chasing the target.
    pub fn tick(&mut self, stack: HeldStack, attack_speed: f64, hands_busy: bool) {
        self.prev_height = self.height;
        self.attack_speed = attack_speed;
        // `Player.tick`: both tickers advance, and a different item type resets both
        // (`ItemStack.isSameItem`: the type only, not the count or components).
        self.swap_ticker += 1;
        self.attack_strength_ticker += 1;
        if self.last_item != stack.item {
            self.swap_ticker = 0;
            self.attack_strength_ticker = 0;
        }
        self.last_item = stack.item;
        let item = stack;
        // `shouldInstantlyReplaceVisibleItem`: the same stack (type, count and non-ignored
        // components) is shown at once; a different one plays the swap animation (every
        // vanilla item has `hand_animation_on_swap`).
        if self.item != item && !SWAP_ANIMATION {
            self.item = item;
        }
        if hands_busy {
            // `FirstPersonHandsAndItems.tick`'s busy branch: the hand dips 0.4 per tick
            // (clamped to 0) instead of chasing the equip target.
            self.height = clamp(self.height - HEIGHT_STEP, 0.0, 1.0);
        } else {
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
        }
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
        self.item.item
    }

    /// `Player.getAttackStrengthScale(a)`: the attack charge 0..1 over the held item's delay.
    pub fn attack_strength_scale(&self, a: f32) -> f32 {
        clamp(
            (self.attack_strength_ticker as f32 + a) / swap_delay(self.attack_speed),
            0.0,
            1.0,
        )
    }

    /// The attack charge's raw parts for the HUD's attack indicator (`Player
    /// .attackStrengthTicker` and `Player.getCurrentItemAttackStrengthDelay`), mirrored into
    /// Java once per tick; the proxy's `getAttackStrengthScale` computes the scale from them.
    pub fn attack_strength_charge(&self) -> (f32, f32) {
        (
            self.attack_strength_ticker as f32,
            swap_delay(self.attack_speed),
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
    /// player's rotation (yaw, pitch) in degrees at its end, `item` the main-hand stack and
    /// `attack_speed` the player's `minecraft:attack_speed` attribute. `hands_busy` is vanilla
    /// `LocalPlayer.rideTick`'s `handsBusy`: the boat we steer, a movement key held.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        moved: f32,
        look: (f32, f32),
        on_ground: bool,
        swimming: bool,
        item: HeldStack,
        attack_speed: f64,
        hands_busy: bool,
    ) {
        // `LivingEntity.baseTick` runs before `Player.tick` and both run before
        // `FirstPersonHandsAndItems.tick`, which the walk bob precedes (`LocalPlayer.aiStep`).
        self.swing.tick();
        self.equip.tick(item, attack_speed, hands_busy);
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

    /// The attack charge's raw parts for the HUD's attack indicator (see [`Equip
    /// ::attack_strength_charge`]).
    pub fn attack_strength_charge(&self) -> (f32, f32) {
        self.equip.attack_strength_charge()
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
mod tests;
