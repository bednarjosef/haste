//! Particle options (`Particle` in metadata and `level_particles`): a
//! `minecraft:particle_type` id followed by type-specific data, laid out per
//! the minecraft.wiki "Java Edition protocol/Particles" page. The data has no
//! length prefix, so it is skipped by type name and kept raw.

use bytes::Bytes;

use super::{DecodeError, DecodeResult, Reader, Wire, Writer};

/// A particle: type id and its raw data.
#[derive(Clone, Debug, PartialEq)]
pub struct Particle {
    /// Index into `minecraft:particle_type`.
    pub kind: i32,
    /// Type-specific data as on the wire.
    pub data: Bytes,
}

impl Wire for Particle {
    fn read(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let kind = r.var_int()?;
        let name = r
            .tables()
            .ok_or_else(|| DecodeError::Invalid(format!("particle {kind} needs registry tables")))?
            .particle_name(kind)
            .ok_or_else(|| DecodeError::Invalid(format!("unknown particle type id {kind}")))?;
        let start = r.peek_rest();
        skip_data(r, name)?;
        let used = start.len() - r.remaining();
        Ok(Particle { kind, data: r.to_bytes(&start[..used]) })
    }
    fn write(&self, w: &mut Writer) {
        w.var_int(self.kind);
        w.bytes(&self.data);
    }
}

/// Skips the data of particle type `name`.
pub fn skip_data(r: &mut Reader<'_>, name: &str) -> DecodeResult<()> {
    let short = name.strip_prefix("minecraft:").unwrap_or(name);
    match short {
        "block" | "block_marker" | "falling_dust" | "dust_pillar" | "block_crumble" | "shriek" => {
            r.var_int()?;
        }
        "dragon_breath" | "sculk_charge" => {
            r.f32()?;
        }
        "geyser" | "geyser_plume" | "entity_effect" | "tinted_leaves" | "flash" => {
            r.i32()?;
        }
        "geyser_base" | "geyser_poof" | "dust" | "effect" | "instant_effect" => {
            r.i32()?;
            r.f32()?;
        }
        "dust_color_transition" => {
            r.i32()?;
            r.i32()?;
            r.f32()?;
        }
        "item" => {
            haste_core::entity::ItemStackData::read(r)?;
        }
        "vibration" => {
            match r.var_int()? {
                0 => {
                    r.block_pos()?;
                }
                1 => {
                    r.var_int()?;
                    r.f32()?;
                }
                t => return Err(DecodeError::Invalid(format!("vibration position source type {t}"))),
            }
            r.var_int()?;
        }
        "trail" => {
            r.f64()?;
            r.f64()?;
            r.f64()?;
            r.i32()?;
            r.var_int()?;
        }
        // Types the wiki lists as data-less.
        "angry_villager" | "bubble" | "sulfur_bubbles" | "noxious_gas" | "noxious_gas_cloud" | "cloud"
        | "copper_fire_flame" | "crit" | "damage_indicator" | "dripping_lava" | "falling_lava" | "landing_lava"
        | "dripping_water" | "falling_water" | "elder_guardian" | "enchanted_hit" | "enchant" | "end_rod"
        | "explosion_emitter" | "explosion" | "gust" | "small_gust" | "gust_emitter_large" | "gust_emitter_small"
        | "sonic_boom" | "firework" | "fishing" | "flame" | "infested" | "cherry_leaves" | "pale_oak_leaves"
        | "sculk_soul" | "sculk_charge_pop" | "soul_fire_flame" | "soul" | "happy_villager" | "composter" | "heart"
        | "pause_mob_growth" | "reset_mob_growth" | "item_slime" | "item_cobweb" | "item_snowball" | "large_smoke"
        | "lava" | "mycelium" | "note" | "poof" | "portal" | "rain" | "smoke" | "white_smoke" | "sneeze" | "spit"
        | "squid_ink" | "sweep_attack" | "totem_of_undying" | "underwater" | "splash" | "witch" | "bubble_pop"
        | "current_down" | "bubble_column_up" | "nautilus" | "dolphin" | "campfire_cosy_smoke"
        | "campfire_signal_smoke" | "dripping_honey" | "falling_honey" | "landing_honey" | "falling_nectar"
        | "falling_spore_blossom" | "ash" | "crimson_spore" | "warped_spore" | "spore_blossom_air"
        | "dripping_obsidian_tear" | "falling_obsidian_tear" | "landing_obsidian_tear" | "reverse_portal"
        | "white_ash" | "small_flame" | "snowflake" | "dripping_dripstone_lava" | "falling_dripstone_lava"
        | "dripping_dripstone_water" | "falling_dripstone_water" | "glow_squid_ink" | "glow" | "wax_on" | "wax_off"
        | "electric_spark" | "scrape" | "egg_crack" | "dust_plume" | "trial_spawner_detection"
        | "trial_spawner_detection_ominous" | "vault_connection" | "ominous_spawning" | "raid_omen" | "trial_omen"
        | "firefly" | "sulfur_cube_goo" => {}
        _ => return Err(DecodeError::Invalid(format!("particle {name} data layout not determined"))),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::ProtocolTables;

    #[test]
    fn particles_round_trip_and_skip() {
        let t = ProtocolTables::default().with_particles_for_tests(&[(0, "minecraft:angry_villager"), (1, "minecraft:block"), (2, "minecraft:dust"), (3, "minecraft:red_poplar_leaves")]);
        let mut w = Writer::new();
        w.write(&Particle { kind: 1, data: Bytes::from_static(&[0x81, 0x01]) });
        w.write(&Particle { kind: 2, data: Bytes::from_static(&[0, 0, 0, 255, 0x3f, 0x80, 0, 0]) });
        w.write(&Particle { kind: 0, data: Bytes::new() });
        let mut r = Reader::new(&w.buf).with_tables(&t);
        assert_eq!(r.read::<Particle>().unwrap().data.len(), 2);
        assert_eq!(r.read::<Particle>().unwrap().data.len(), 8);
        assert_eq!(r.read::<Particle>().unwrap().kind, 0);
        r.finish().unwrap();
        assert!(Reader::new(&[3]).with_tables(&t).read::<Particle>().is_err());
    }
}
