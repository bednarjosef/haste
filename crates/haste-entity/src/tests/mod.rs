//! The crate's tests, one file per topic; the shared fixtures live here.
use std::time::Instant;

use haste_core::entity::{EntitySpawn, MetadataEntry, MetadataValue, PlayerInfo, Pose};
use haste_core::game::GameMode;
use haste_core::ids::{EntityId, RegistryIndex};
use haste_core::model::{BodyOp, EntityInstance};
use haste_core::DVec3;
use uuid::Uuid;

use crate::dims::EntityDimensions;
use crate::riding::{passenger_seat, PassengerKind, SeatPoint, SeatQuery};
use crate::store::{Entity, EntityStore};
use haste_core::model::{CubeModel, ModelPart};

mod animation;
mod connectivity;
mod hurt;
mod mob_animation;
mod model_dump;
mod models;
mod registry;
mod riding;
mod setup_rotations;
mod store;
fn store() -> EntityStore {
    EntityStore::new(EntityDimensions::default())
}

fn spawn(id: i32, pos: DVec3) -> EntitySpawn {
    EntitySpawn {
        id: EntityId(id),
        uuid: Uuid::new_v4(),
        kind: RegistryIndex(0),
        pos,
        yaw: 0.0,
        pitch: 0.0,
        head_yaw: 0.0,
        data: 0,
        velocity: DVec3::ZERO,
    }
}

fn find_part<'a>(p: &'a ModelPart, name: &str) -> Option<&'a ModelPart> {
    if p.name == name {
        Some(p)
    } else {
        p.children.iter().find_map(|c| find_part(c, name))
    }
}

fn humanoid_parts(m: &CubeModel) -> Vec<&str> {
    let mut v: Vec<&str> = m.root.children.iter().map(|c| c.name.as_str()).collect();
    v.extend(
        m.root
            .children
            .iter()
            .flat_map(|c| c.children.iter().map(|g| g.name.as_str())),
    );
    v
}

/// Builds a minimal `CubeModel` whose root has one cube-less child per `parts` name, so the
/// animation classification (by part name) and setupAnim can be exercised without the dump.
fn mob_model(texture: &str, parts: &[&str]) -> CubeModel {
    let children = parts
        .iter()
        .map(|name| ModelPart {
            name: (*name).to_owned(),
            pivot: glam::Vec3::ZERO,
            rotation: glam::Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children: Vec::new(),
        })
        .collect();
    CubeModel {
        texture: haste_core::Identifier::minecraft(texture),
        tex_width: 64,
        tex_height: 64,
        root: ModelPart {
            name: "root".to_owned(),
            pivot: glam::Vec3::ZERO,
            rotation: glam::Vec3::ZERO,
            visible: true,
            cubes: Vec::new(),
            children,
        },
    }
}

/// Every living entity type vanilla registers a mob renderer for (EntityRenderers.java's
/// PROVIDERS, minus the player/mannequin handled separately, minus non-living types).
/// A type appearing here but not in the table (and not in NON_LIVING) fails the coverage
/// test, so new mob types cannot silently fall back to the box.
const LIVING_TYPES: &[&str] = &[
    "minecraft:allay",
    "minecraft:armadillo",
    "minecraft:axolotl",
    "minecraft:bat",
    "minecraft:bee",
    "minecraft:blaze",
    "minecraft:bogged",
    "minecraft:breeze",
    "minecraft:camel",
    "minecraft:camel_husk",
    "minecraft:cat",
    "minecraft:cave_spider",
    "minecraft:chicken",
    "minecraft:cod",
    "minecraft:copper_golem",
    "minecraft:cow",
    "minecraft:creaking",
    "minecraft:creeper",
    "minecraft:dolphin",
    "minecraft:donkey",
    "minecraft:drowned",
    "minecraft:elder_guardian",
    "minecraft:ender_dragon",
    "minecraft:enderman",
    "minecraft:endermite",
    "minecraft:evoker",
    "minecraft:fox",
    "minecraft:frog",
    "minecraft:ghast",
    "minecraft:giant",
    "minecraft:glow_squid",
    "minecraft:goat",
    "minecraft:guardian",
    "minecraft:happy_ghast",
    "minecraft:hoglin",
    "minecraft:horse",
    "minecraft:husk",
    "minecraft:illusioner",
    "minecraft:iron_golem",
    "minecraft:llama",
    "minecraft:magma_cube",
    "minecraft:mooshroom",
    "minecraft:mule",
    "minecraft:nautilus",
    "minecraft:ocelot",
    "minecraft:panda",
    "minecraft:parched",
    "minecraft:parrot",
    "minecraft:phantom",
    "minecraft:pig",
    "minecraft:piglin",
    "minecraft:piglin_brute",
    "minecraft:pillager",
    "minecraft:polar_bear",
    "minecraft:pufferfish",
    "minecraft:rabbit",
    "minecraft:ravager",
    "minecraft:salmon",
    "minecraft:sheep",
    "minecraft:shulker",
    "minecraft:silverfish",
    "minecraft:skeleton",
    "minecraft:skeleton_horse",
    "minecraft:slime",
    "minecraft:sniffer",
    "minecraft:snow_golem",
    "minecraft:spider",
    "minecraft:squid",
    "minecraft:stray",
    "minecraft:strider",
    "minecraft:sulfur_cube",
    "minecraft:tadpole",
    "minecraft:trader_llama",
    "minecraft:tropical_fish",
    "minecraft:turtle",
    "minecraft:vex",
    "minecraft:villager",
    "minecraft:vindicator",
    "minecraft:wandering_trader",
    "minecraft:warden",
    "minecraft:witch",
    "minecraft:wither",
    "minecraft:wither_skeleton",
    "minecraft:wolf",
    "minecraft:zoglin",
    "minecraft:zombie",
    "minecraft:zombie_horse",
    "minecraft:zombie_nautilus",
    "minecraft:zombie_villager",
    "minecraft:zombified_piglin",
];

/// Entity types with renderers but no living-model layer: projectiles, vehicles, block-like
/// and UI entities (out of scope; drawn as boxes) plus the player (its own model path).
#[allow(dead_code)] // documents what the table deliberately leaves out
const NON_LIVING: &[&str] = &[
    "boat-like: *_boat, *_chest_boat, *_raft",
    "minecart-like: minecart, *_minecart, tnt_minecart",
    "projectiles: arrow, egg, ender_pearl, trident, fireball(s), snowball, potions,",
    "  dragon_fireball, evoker_fangs, llama_spit, shulker_bullet, wind_charge(s),",
    "  experience_bottle, eye_of_ender, fishing_bobber, wither_skull",
    "object entities: area_effect_cloud, armor_stand, block/item/text_display, cushion,",
    "  end_crystal, falling_block, firework_rocket, interaction, item, item_frame(s),",
    "  leash_knot, lightning_bolt, mannequin, marker, ominous_item_spawner, painting,",
    "  player, tnt, experience_orb",
];
