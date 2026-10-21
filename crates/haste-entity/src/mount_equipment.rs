//! The mobs' own equipment layers, which draw a worn item's equipment asset on a layer model of
//! the wearer's renderer instead of the humanoid armour set:
//! - `SimpleEquipmentLayer`, as `HorseRenderer`, `UndeadHorseRenderer`, `DonkeyRenderer`,
//!   `CamelRenderer`, `CamelHuskRenderer`, `PigRenderer`, `StriderRenderer`, `NautilusRenderer`
//!   and `ZombieNautilusRenderer` add it for the body armour and the saddle;
//! - `WolfArmorLayer` (the wolf's body armour and its crack overlay, `maybeRenderCracks`);
//! - `LlamaDecorLayer` (a llama's carpet, the trader llama's own decor without one).
//!
//! The table is keyed by entity type id like vanilla's `EntityRenderers` registrations; a type
//! it does not name (a mod's) wears nothing here. Each row resolves to an `EquipmentPiece`
//! carrying the layer model and layer type, which the renderer draws through the asset's
//! layers (`EquipmentLayerRenderer.renderLayers`).
use std::sync::OnceLock;

use haste_core::entity::ItemStackData;
use haste_core::model::{EquipmentModelLayer, EquipmentPiece, EquipmentSlot, PoseOverrides};
use haste_core::parts::{self, PartId};
use haste_core::Identifier;
use haste_data::ItemComponents;

use crate::model_dump;

/// How a layer decides what it draws (which vanilla layer class it follows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LayerKind {
    /// `SimpleEquipmentLayer.submit`: the slot's item when it has an equipment asset; on a
    /// baby only when the layer has a baby model.
    Simple,
    /// `WolfArmorLayer.submit`: the body item on adults, then its cracks.
    WolfArmor,
    /// `LlamaDecorLayer.submit`: the body item on adults; otherwise a trader llama's own decor
    /// asset (`EquipmentAssets.TRADER_LLAMA` / `TRADER_LLAMA_BABY`) on the matching model.
    LlamaDecor { trader: bool },
}

/// One equipment layer a mob renderer adds: the slot it reads, its layer models (the dumped
/// `ModelLayers` entry for adults and, when the renderer bakes one, babies), the
/// `EquipmentClientInfo.LayerType` its asset textures are keyed by, and its rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LayerRow {
    pub slot: EquipmentSlot,
    pub adult: &'static str,
    pub baby: Option<&'static str>,
    pub layer_type: &'static str,
    pub kind: LayerKind,
}

const fn simple(slot: EquipmentSlot, adult: &'static str, layer_type: &'static str) -> LayerRow {
    LayerRow {
        slot,
        adult,
        baby: None,
        layer_type,
        kind: LayerKind::Simple,
    }
}

const fn llama(trader: bool) -> LayerRow {
    LayerRow {
        slot: EquipmentSlot::Body,
        adult: "minecraft:llama#decor",
        baby: Some("minecraft:llama_baby#decor"),
        layer_type: "llama_body",
        kind: LayerKind::LlamaDecor { trader },
    }
}

use EquipmentSlot::{Body, Saddle};

const HORSE: &[LayerRow] = &[
    simple(Body, "minecraft:horse_armor#main", "horse_body"),
    simple(Saddle, "minecraft:horse#saddle", "horse_saddle"),
];
const SKELETON_HORSE: &[LayerRow] = &[
    simple(Body, "minecraft:undead_horse_armor#main", "horse_body"),
    simple(
        Saddle,
        "minecraft:skeleton_horse#saddle",
        "skeleton_horse_saddle",
    ),
];
const ZOMBIE_HORSE: &[LayerRow] = &[
    simple(Body, "minecraft:undead_horse_armor#main", "horse_body"),
    simple(
        Saddle,
        "minecraft:zombie_horse#saddle",
        "zombie_horse_saddle",
    ),
];
const DONKEY: &[LayerRow] = &[simple(Saddle, "minecraft:donkey#saddle", "donkey_saddle")];
const MULE: &[LayerRow] = &[simple(Saddle, "minecraft:mule#saddle", "mule_saddle")];
const CAMEL: &[LayerRow] = &[simple(Saddle, "minecraft:camel#saddle", "camel_saddle")];
const CAMEL_HUSK: &[LayerRow] = &[simple(
    Saddle,
    "minecraft:camel_husk#saddle",
    "camel_husk_saddle",
)];
const PIG: &[LayerRow] = &[simple(Saddle, "minecraft:pig#saddle", "pig_saddle")];
const STRIDER: &[LayerRow] = &[simple(Saddle, "minecraft:strider#saddle", "strider_saddle")];
const NAUTILUS: &[LayerRow] = &[
    simple(Body, "minecraft:nautilus_armor#main", "nautilus_body"),
    simple(Saddle, "minecraft:nautilus#saddle", "nautilus_saddle"),
];
const WOLF: &[LayerRow] = &[LayerRow {
    slot: Body,
    adult: "minecraft:wolf_armor#main",
    baby: None,
    layer_type: "wolf_body",
    kind: LayerKind::WolfArmor,
}];
const LLAMA: &[LayerRow] = &[llama(false)];
const TRADER_LLAMA: &[LayerRow] = &[llama(true)];

/// The equipment layers an entity type's renderer adds, in its `addLayer` order
/// (`EntityRenderers` registrations). Empty for every other type.
pub(crate) fn layers_for(type_name: &str) -> &'static [LayerRow] {
    match type_name {
        "minecraft:horse" => HORSE,
        "minecraft:skeleton_horse" => SKELETON_HORSE,
        "minecraft:zombie_horse" => ZOMBIE_HORSE,
        "minecraft:donkey" => DONKEY,
        "minecraft:mule" => MULE,
        "minecraft:camel" => CAMEL,
        "minecraft:camel_husk" => CAMEL_HUSK,
        "minecraft:pig" => PIG,
        "minecraft:strider" => STRIDER,
        "minecraft:nautilus" | "minecraft:zombie_nautilus" => NAUTILUS,
        "minecraft:wolf" => WOLF,
        "minecraft:llama" => LLAMA,
        "minecraft:trader_llama" => TRADER_LLAMA,
        _ => &[],
    }
}

/// What one row draws for a wearer: the layer model id, the equipment asset, and the stack
/// the dye and wear come from (`None` for the trader llama's own decor). `None` when the row
/// draws nothing.
pub(crate) fn resolve<'a>(
    row: &LayerRow,
    baby: bool,
    stack: Option<&'a ItemStackData>,
    components: &ItemComponents,
) -> Option<(&'static str, Identifier, Option<&'a ItemStackData>)> {
    let stack = stack.filter(|s| !s.is_empty());
    // The item's `equippable` asset; an item without one draws nothing (all three layers check
    // `equippable.assetId()` first).
    let asset = stack
        .and_then(|s| s.item)
        .and_then(|i| components.equippable(i))
        .and_then(|e| e.asset.clone());
    match row.kind {
        LayerKind::Simple => {
            let model = if baby { row.baby? } else { row.adult };
            Some((model, asset?, stack))
        }
        LayerKind::WolfArmor if baby => None,
        LayerKind::WolfArmor => Some((row.adult, asset?, stack)),
        LayerKind::LlamaDecor { trader } => match asset {
            Some(asset) if !baby => Some((row.adult, asset, stack)),
            _ if trader => {
                let (model, asset) = if baby {
                    (row.baby?, "trader_llama_baby")
                } else {
                    (row.adult, "trader_llama")
                };
                Some((model, Identifier::minecraft(asset), None))
            }
            _ => None,
        },
    }
}

/// The wearer's mob equipment pieces, in layer order. Allocates only for a mob that wears
/// something (or a trader llama); every other entity gets an empty `Vec`.
pub(crate) fn pieces(
    type_name: Option<&str>,
    baby: bool,
    equipment: &[(u8, ItemStackData)],
    components: &ItemComponents,
) -> Vec<EquipmentPiece> {
    let rows = type_name.map_or(&[][..], layers_for);
    let mut out = Vec::new();
    for row in rows {
        let stack = equipment
            .iter()
            .find(|(wire, _)| EquipmentSlot::from_wire(*wire) == Some(row.slot))
            .map(|(_, s)| s);
        let Some((layer, asset, stack)) = resolve(row, baby, stack, components) else {
            continue;
        };
        let Some(model) = model_dump::library().and_then(|lib| lib.get(layer, 1.0)) else {
            continue;
        };
        let item = stack.and_then(|s| s.item);
        let cracks = match (row.kind, stack, item) {
            (LayerKind::WolfArmor, Some(s), Some(i)) => {
                wolf_armor_cracks(components.wear(i, &s.components_raw))
            }
            _ => None,
        };
        out.push(EquipmentPiece {
            slot: row.slot,
            item,
            asset: Some(asset),
            dye: stack.and_then(|s| components.dyed_color(&s.components_raw)),
            layer: Some(EquipmentModelLayer {
                model,
                layer_type: row.layer_type,
                cracks,
            }),
        });
    }
    out
}

/// The saddle models' own `setupAnim` on top of the wearer's: `EquineSaddleModel` shows its
/// reins (`left_saddle_line`, `right_saddle_line`) and `CamelSaddleModel` its `reins` only while
/// ridden (`state.isRidden`, `Entity.isVehicle`). The wearer's model has no such parts, so
/// hiding them in the shared pose touches only the saddle.
pub(crate) fn saddle_setup_anim(type_name: Option<&str>, ridden: bool, pose: &mut PoseOverrides) {
    if ridden {
        return;
    }
    static REINS: OnceLock<[PartId; 3]> = OnceLock::new();
    let [left, right, camel] = *REINS.get_or_init(|| {
        [
            parts::intern("left_saddle_line"),
            parts::intern("right_saddle_line"),
            parts::intern("reins"),
        ]
    });
    match type_name {
        Some(
            "minecraft:horse"
            | "minecraft:skeleton_horse"
            | "minecraft:zombie_horse"
            | "minecraft:donkey"
            | "minecraft:mule",
        ) => {
            pose.hide(left);
            pose.hide(right);
        }
        Some("minecraft:camel" | "minecraft:camel_husk") => pose.hide(camel),
        _ => {}
    }
}

/// `WolfArmorLayer.maybeRenderCracks`' texture for the armour's wear (`(damage, max_damage)`,
/// `None` when the stack is not damageable): `Crackiness.WOLF_ARMOR.byDamage`'s level as its
/// `ARMOR_CRACK_LOCATIONS` texture, `None` at `Level.NONE`.
pub(crate) fn wolf_armor_cracks(wear: Option<(u32, u32)>) -> Option<Identifier> {
    let (damage, max) = wear?;
    crackiness_texture(cracked_fraction(max, damage)?).map(Identifier::minecraft)
}

/// `Crackiness.byDamage(damage, maxDamage)`'s remaining fraction:
/// `(maxDamage - damage) / maxDamage`. `None` for a max of 0 (nothing to divide by).
pub(crate) fn cracked_fraction(max: u32, damage: u32) -> Option<f32> {
    if max == 0 {
        return None;
    }
    Some((max as i64 - damage as i64) as f32 / max as f32)
}

/// `Crackiness.WOLF_ARMOR.byFraction` as the crack texture it selects: high below 0.32,
/// medium below 0.69, low below 0.95, none from 0.95 up.
pub(crate) fn crackiness_texture(fraction: f32) -> Option<&'static str> {
    if fraction < 0.32 {
        Some("entity/wolf/wolf_armor_crackiness_high")
    } else if fraction < 0.69 {
        Some("entity/wolf/wolf_armor_crackiness_medium")
    } else if fraction < 0.95 {
        Some("entity/wolf/wolf_armor_crackiness_low")
    } else {
        None
    }
}
