//! `equipment::layer_tint`: vanilla `EquipmentLayerRenderer.getColorForLayer` (a dyeable layer
//! takes the stack's dye, else its undyed colour, else is skipped; other layers draw white).
use haste_assets::{Dyeable, EquipmentLayer};
use haste_core::Identifier;

use crate::entity::equipment::layer_tint;
use crate::entity::item_display::argb_linear;

fn layer(dyeable: Option<Dyeable>) -> EquipmentLayer {
    EquipmentLayer {
        layer_type: "horse_body".into(),
        texture: Identifier::minecraft("textures/entity/equipment/horse_body/leather.png"),
        dyeable,
        use_player_texture: false,
    }
}

#[test]
fn dyeable_layer_takes_the_dye_then_the_undyed_colour() {
    // Leather horse armour's base layer: `color_when_undyed` -6265536 (0xFFA06540).
    let leather = layer(Some(Dyeable {
        color_when_undyed: Some(0xFFA0_6540),
    }));
    assert_eq!(layer_tint(&leather, None), Some(argb_linear(0xFFA0_6540)));
    // A dyed stack's rgb (the component stores no alpha) is made opaque and wins.
    assert_eq!(
        layer_tint(&leather, Some(0x00B0_2E26)),
        Some(argb_linear(0xFFB0_2E26))
    );
    // Black dye is still a colour, not vanilla's "skip" 0.
    assert_eq!(layer_tint(&leather, Some(0)), Some([0.0; 3]));
}

#[test]
fn dyeable_layer_without_undyed_colour_is_skipped_until_dyed() {
    // Wolf armour's overlay (`"dyeable": {}`): drawn only when the armour is dyed.
    let overlay = layer(Some(Dyeable {
        color_when_undyed: None,
    }));
    assert_eq!(layer_tint(&overlay, None), None);
    assert_eq!(
        layer_tint(&overlay, Some(0x003A_B3DA)),
        Some(argb_linear(0xFF3A_B3DA))
    );
}

#[test]
fn non_dyeable_layer_ignores_the_dye() {
    // Leather armour's overlay layer has no `dyeable`: white even on dyed armour.
    let plain = layer(None);
    assert_eq!(layer_tint(&plain, None), Some([1.0; 3]));
    assert_eq!(layer_tint(&plain, Some(0x00B0_2E26)), Some([1.0; 3]));
}
