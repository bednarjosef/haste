//! What the first-person view holds (vanilla `FirstPersonHandsAndItemsRenderState`): the
//! local player's hand state the renderer draws the held item and arm from.

use super::TickPair;
use haste_core::Identifier;

/// What the first-person view holds: the local player's skin and arm model, the item in the
/// main hand (the arm is drawn only when the hand is empty, like vanilla), the light at the
/// camera, and this tick's hand animation (swing, equip progress and bob).
///
/// Everything animated lives here as the two ends of the last tick; the renderer lerps them by
/// partial tick and applies vanilla's transforms, so all per-tick state stays in the client.
#[derive(Clone, Debug, PartialEq)]
pub struct FirstPerson {
    pub skin: Option<Identifier>,
    pub slim: bool,
    /// `AvatarRenderState.showRightSleeve`: the arm draws its sleeve layer.
    pub sleeve: bool,
    pub item: Option<haste_core::RegistryIndex>,
    pub light: (u8, u8),
    /// The main hand is using an item with the EAT or DRINK animation and ticks left
    /// (`FirstPersonHandsAndItemsRenderState`: `isUsingItem && useItemRemainingTicks > 0 &&
    /// useItemHand == MAIN_HAND`): the eat/drink transform replaces the swing.
    pub use_eat: bool,
    /// `FirstPersonHandsAndItemsRenderState.useItemRemainingTicks` (`getUseItemRemainingTicks`).
    pub use_remaining: i32,
    /// `FirstPersonHandsAndItemsRenderState.mainHandUseDuration` (`Item.getUseDuration`, a
    /// consumable's `consume_ticks`).
    pub use_duration: i32,
    /// Swing progress 0..1 while the main hand swings, 0 when it does not
    /// (`FirstPersonHandsAndItemsRenderState`'s `attackValue`).
    pub attack: TickPair,
    /// Equip progress: 1 when the item is fully held, 0 while it switches
    /// (`FirstPersonHandsAndItems.mainHandHeight`, the inverse of vanilla's `inverseArmHeight`).
    pub equip: TickPair,
    /// Hand lag behind the camera turn, in degrees (`LocalPlayer.xBob`/`xBobO`).
    pub x_bob: TickPair,
    /// Hand lag behind the camera turn, in degrees (`LocalPlayer.yBob`/`yBobO`).
    pub y_bob: TickPair,
}

impl Default for FirstPerson {
    /// The empty hand at rest: fully equipped, no swing, no use, no bob, full block light.
    fn default() -> Self {
        FirstPerson {
            skin: None,
            slim: false,
            sleeve: true,
            item: None,
            light: (15, 0),
            use_eat: false,
            use_remaining: 0,
            use_duration: 0,
            attack: TickPair::fixed(0.0),
            equip: TickPair::fixed(1.0),
            x_bob: TickPair::fixed(0.0),
            y_bob: TickPair::fixed(0.0),
        }
    }
}
