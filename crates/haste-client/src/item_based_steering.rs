//! Vanilla's `ItemBasedSteering` (the boost a food-on-a-stick click starts) and the animals
//! steered that way: the pig with a carrot on a stick ([`crate::pig`]) and the strider with a
//! warped fungus on a stick ([`crate::strider`]). Both walk a constant full forward while the
//! rider holds their stick (`getRiddenInput`, `getControllingPassenger`), and the rider only
//! steers with the look.
use glam::DVec3;
use haste_core::entity::{MetadataEntry, MetadataValue};

/// Vanilla `ItemBasedSteering.boostFactor`: `1 + 1.15 * sin(boostTime / total * PI)`.
const BOOST_AMPLITUDE: f32 = 1.15;

/// The boost state of one ridden animal (vanilla `ItemBasedSteering`). The total is the
/// server's: `boost` writes a random 140..981 into the animal's synched `DATA_BOOST_TIME`, and
/// the metadata update starts our side of it (`onSynced`).
#[derive(Default)]
pub struct ItemBasedSteering {
    boosting: bool,
    boost_time: i32,
    boost_time_total: i32,
}

impl ItemBasedSteering {
    /// Vanilla `onSynced`: a change of the synched boost total starts the boost from zero.
    pub fn on_synced(&mut self, total: i32) {
        self.boosting = true;
        self.boost_time = 0;
        self.boost_time_total = total;
    }

    /// Vanilla `tickBoost`: `boostTime++ > total` compares the time before the increment.
    pub fn tick_boost(&mut self) {
        if self.boosting {
            let before = self.boost_time;
            self.boost_time += 1;
            if before > self.boost_time_total {
                self.boosting = false;
            }
        }
    }

    /// Vanilla `boostFactor`.
    pub fn boost_factor(&self) -> f32 {
        if self.boosting && self.boost_time_total > 0 {
            1.0 + BOOST_AMPLITUDE
                * (self.boost_time as f32 / self.boost_time_total as f32 * std::f32::consts::PI)
                    .sin()
        } else {
            1.0
        }
    }

    #[cfg(test)]
    pub fn boosting(&self) -> bool {
        self.boosting
    }
}

/// The synched data index of `DATA_BOOST_TIME` for both animals (the first accessor after
/// `AgeableMob`'s: `Entity` 8, `LivingEntity` 7, `Mob` 1, `AgeableMob` 2).
pub const DATA_BOOST_TIME: u8 = 18;

/// The boost's total from a `set_entity_data` update (`DATA_BOOST_TIME`, a VarInt), if the
/// update carries it.
pub fn boost_total_of(entries: &[MetadataEntry]) -> Option<i32> {
    entries
        .iter()
        .find(|e| e.index == DATA_BOOST_TIME)
        .and_then(|e| match &e.value {
            MetadataValue::VarInt(v) => Some(*v),
            _ => None,
        })
}

/// The stick an item-steered animal answers to (its `getControllingPassenger` item).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SteerStick {
    /// `Items.CARROT_ON_A_STICK` (the pig).
    CarrotOnAStick,
    /// `Items.WARPED_FUNGUS_ON_A_STICK` (the strider).
    WarpedFungusOnAStick,
}

impl SteerStick {
    /// The stick's item registry path.
    pub fn item_path(self) -> &'static str {
        match self {
            SteerStick::CarrotOnAStick => "carrot_on_a_stick",
            SteerStick::WarpedFungusOnAStick => "warped_fungus_on_a_stick",
        }
    }
}

/// The item-steered animal we ride: one of vanilla's two `ItemSteerable` mobs, simulated on our
/// side while we hold its stick.
pub enum ItemSteered {
    Pig(crate::pig::Pig),
    Strider(crate::strider::Strider),
}

/// `#minecraft:strider_warm_blocks` (vanilla `BlockTags.STRIDER_WARM_BLOCKS`), the tag the
/// strider's warm/cold test reads.
pub fn strider_warm_blocks() -> haste_core::Identifier {
    haste_core::Identifier::minecraft("strider_warm_blocks")
}

/// Resolves `#strider_warm_blocks` for a block state, as the driver wires it from the synced
/// block tags and the game data's block table. Implemented once by `net.rs`.
pub trait WarmBlocks {
    /// Whether `state`'s block is in the tag.
    fn is_strider_warm(&self, state: haste_core::BlockStateId) -> bool;
}

/// What an item-steered animal's tick reads from the world.
pub struct SteerEnv<'a> {
    pub world: &'a dyn haste_core::view::BlockView,
    pub shapes: &'a dyn haste_core::view::CollisionShapes,
    pub blocks: &'a dyn haste_core::view::BlockPhysics,
    /// The `#strider_warm_blocks` resolution (synced tags + block table); `None` without a
    /// tag sync yet, in which case no block warms (lava alone still does, by fluid height).
    pub warm_blocks: Option<&'a dyn WarmBlocks>,
}

impl ItemSteered {
    /// The stick this animal answers to.
    pub fn stick(&self) -> SteerStick {
        match self {
            ItemSteered::Pig(_) => SteerStick::CarrotOnAStick,
            ItemSteered::Strider(_) => SteerStick::WarpedFungusOnAStick,
        }
    }

    /// (pos, yaw, pitch, on_ground) of our simulation, what `move_vehicle` carries.
    pub fn pose(&self) -> (DVec3, f32, f32, bool) {
        match self {
            ItemSteered::Pig(p) => (p.pos, p.yaw, p.pitch, p.on_ground),
            ItemSteered::Strider(s) => (s.pos, s.yaw, s.pitch, s.on_ground),
        }
    }

    pub fn tick(&mut self, env: &SteerEnv<'_>) {
        match self {
            ItemSteered::Pig(p) => p.tick(env),
            ItemSteered::Strider(s) => s.tick(env),
        }
    }

    pub fn set_look(&mut self, look: (f32, f32)) {
        match self {
            ItemSteered::Pig(p) => p.set_look(look),
            ItemSteered::Strider(s) => s.set_look(look),
        }
    }

    /// The server's correction (`handleMoveVehicle`), and the pose the server-driven animal
    /// has while we do not hold its stick (it is not ours to simulate then).
    pub fn correct(&mut self, pos: DVec3, yaw: f32) {
        match self {
            ItemSteered::Pig(p) => p.correct(pos, yaw),
            ItemSteered::Strider(s) => s.correct(pos, yaw),
        }
    }

    /// While we do not control it (vanilla: no controlling passenger, so it is interpolated
    /// from the server's moves): follow the server's pose, with no momentum of our own.
    pub fn follow(&mut self, pos: DVec3, yaw: f32) {
        self.correct(pos, yaw);
        match self {
            ItemSteered::Pig(p) => p.delta = DVec3::ZERO,
            ItemSteered::Strider(s) => s.delta = DVec3::ZERO,
        }
    }

    pub fn steering_mut(&mut self) -> &mut ItemBasedSteering {
        match self {
            ItemSteered::Pig(p) => &mut p.steering,
            ItemSteered::Strider(s) => &mut s.steering,
        }
    }

    /// A `set_entity_data` for this animal: the boost total (both) and the strider's
    /// suffocating flag.
    pub fn on_metadata(&mut self, entries: &[MetadataEntry]) {
        if let Some(total) = boost_total_of(entries) {
            tracing::debug!("ridden animal boost started: {total} ticks");
            self.steering_mut().on_synced(total);
        }
        if let ItemSteered::Strider(s) = self {
            s.on_metadata(entries);
        }
    }
}
