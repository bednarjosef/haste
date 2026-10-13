//! Packet structs, one per `minecraft:` packet in `packets.json`, grouped by
//! state and direction. Every struct implements [`Packet`] (decode and encode,
//! so each one round-trips in tests). Per-state enums ([`play::ClientboundPlay`]
//! and friends) dispatch on the runtime id table from [`ids::PacketIds`].
//!
//! Field layouts follow the minecraft.wiki page "Java Edition protocol/Packets"
//! for 26.3 (protocol 777); where the page and the server disagree, NOTES.md
//! says what was verified against the local server.

use bytes::Bytes;

use crate::wire::{DecodeError, DecodeResult, Reader, Writer};

pub mod configuration;
pub mod handshake;
pub mod ids;
pub mod login;
pub mod play;
pub mod status;

pub use ids::{Direction, PacketIds, ProtocolTables, State};

/// A packet body codec.
pub trait Packet: Sized {
    /// The packet's name in `packets.json`, e.g. `minecraft:login`.
    const NAME: &'static str;
    /// Decodes the body (after the packet id).
    fn decode(r: &mut Reader<'_>) -> DecodeResult<Self>;
    /// Encodes the body (without the packet id).
    fn encode(&self, w: &mut Writer);

    /// Decodes a complete body and fails if bytes are left over.
    fn decode_exact(r: &mut Reader<'_>) -> DecodeResult<Self> {
        let p = Self::decode(r)?;
        r.finish()?;
        Ok(p)
    }
}

/// Defines a packet struct whose fields are encoded in order. Each field is
/// `name: Type` (canonical [`crate::wire::Wire`] encoding) or
/// `name: Type as Codec` (explicit [`crate::wire::Codec`]).
macro_rules! packet {
    ($(#[$m:meta])* $name:ident = $id:literal { $( $(#[$fm:meta])* $f:ident : $t:ty $(as $c:ty)? ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        pub struct $name { $( $(#[$fm])* pub $f: $t, )* }
        impl $crate::packets::Packet for $name {
            const NAME: &'static str = $id;
            #[allow(unused_variables)]
            fn decode(r: &mut $crate::wire::Reader<'_>) -> $crate::wire::DecodeResult<Self> {
                Ok(Self { $( $f: packet!(@read r, $t $(, $c)?), )* })
            }
            #[allow(unused_variables)]
            fn encode(&self, w: &mut $crate::wire::Writer) {
                $( packet!(@write w, &self.$f, $t $(, $c)?); )*
            }
        }
    };
    (@read $r:ident, $t:ty) => { <$t as $crate::wire::Wire>::read($r)? };
    (@read $r:ident, $t:ty, $c:ty) => { <$c as $crate::wire::Codec<$t>>::read($r)? };
    (@write $w:ident, $v:expr, $t:ty) => { <$t as $crate::wire::Wire>::write($v, $w) };
    (@write $w:ident, $v:expr, $t:ty, $c:ty) => { <$c as $crate::wire::Codec<$t>>::write($v, $w) };
}
pub(crate) use packet;

/// Defines the enum of all packets of one state and direction, with an
/// `Unknown { id, body }` variant, and its dispatch helpers.
macro_rules! packet_enum {
    ($(#[$m:meta])* $enum:ident { $( $variant:ident ),* $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq)]
        #[allow(clippy::large_enum_variant)]
        pub enum $enum {
            $(
                #[doc = concat!("`", stringify!($variant), "`.")]
                $variant($variant),
            )*
            /// A packet we have no decoder for, or whose decode failed; kept raw.
            Unknown {
                /// Wire id.
                id: i32,
                /// Body after the id.
                body: bytes::Bytes,
            },
        }
        impl $enum {
            /// `(name, decoder)` for every packet of this enum.
            pub const DECODERS: &'static [(&'static str, $crate::packets::DecodeFn<$enum>)] = &[
                $( (<$variant as $crate::packets::Packet>::NAME, |r| Ok($enum::$variant(<$variant as $crate::packets::Packet>::decode_exact(r)?))), )*
            ];
            /// The `minecraft:` name, or `None` for `Unknown`.
            pub fn name(&self) -> Option<&'static str> {
                match self {
                    $( $enum::$variant(_) => Some(<$variant as $crate::packets::Packet>::NAME), )*
                    $enum::Unknown { .. } => None,
                }
            }
            /// Writes the body (without id). `Unknown` writes its raw body.
            pub fn encode_body(&self, w: &mut $crate::wire::Writer) {
                match self {
                    $( $enum::$variant(p) => <$variant as $crate::packets::Packet>::encode(p, w), )*
                    $enum::Unknown { body, .. } => w.bytes(body),
                }
            }
        }
        $(
            impl From<$variant> for $enum {
                fn from(p: $variant) -> Self { $enum::$variant(p) }
            }
        )*
    };
}
pub(crate) use packet_enum;

/// A decoder function stored in a dispatch table.
pub type DecodeFn<T> = fn(&mut Reader<'_>) -> DecodeResult<T>;

/// Id-indexed decoder table for one packet enum, built from [`PacketIds`].
#[derive(Clone)]
pub struct Dispatch<T> {
    by_id: Vec<Option<(&'static str, DecodeFn<T>)>>,
}

impl<T> std::fmt::Debug for Dispatch<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Dispatch({} ids)",
            self.by_id.iter().filter(|x| x.is_some()).count()
        )
    }
}

/// The outcome of decoding one packet through a [`Dispatch`].
pub enum Decoded<T> {
    /// Decoded.
    Ok(T),
    /// The id is not in `packets.json` or we have no struct for it.
    NoDecoder,
    /// We have a struct but the body did not match it.
    Failed(&'static str, DecodeError),
}

impl<T> Dispatch<T> {
    /// Builds the table for `state`/`dir` from the ids. Names listed in
    /// `packets.json` without a decoder in `decoders` stay `Unknown`.
    pub fn new(
        ids: &PacketIds,
        state: State,
        dir: Direction,
        decoders: &'static [(&'static str, DecodeFn<T>)],
    ) -> Self {
        let mut by_id = Vec::new();
        for (id, name) in ids.iter(state, dir) {
            if let Some((n, f)) = decoders.iter().find(|(n, _)| *n == name) {
                let i = id as usize;
                if by_id.len() <= i {
                    by_id.resize(i + 1, None);
                }
                by_id[i] = Some((*n, *f));
            }
        }
        Self { by_id }
    }

    /// The packet name for an id, if it has a decoder.
    pub fn name(&self, id: i32) -> Option<&'static str> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.by_id.get(i).copied().flatten())
            .map(|(n, _)| n)
    }

    /// Decodes `body` as packet `id`.
    pub fn decode(&self, id: i32, body: &Bytes, tables: Option<&ProtocolTables>) -> Decoded<T> {
        let Some((name, f)) = usize::try_from(id)
            .ok()
            .and_then(|i| self.by_id.get(i).copied().flatten())
        else {
            return Decoded::NoDecoder;
        };
        let mut r = Reader::from_bytes(body);
        if let Some(t) = tables {
            r = r.with_tables(t);
        }
        match f(&mut r) {
            Ok(p) => Decoded::Ok(p),
            Err(e) => Decoded::Failed(name, e),
        }
    }
}

/// Encodes a packet body with its id from the table.
pub fn encode_with_id<P: Packet>(
    ids: &PacketIds,
    state: State,
    dir: Direction,
    p: &P,
) -> Option<Bytes> {
    let id = ids.id(state, dir, P::NAME)?;
    let mut w = Writer::new();
    w.var_int(id);
    p.encode(&mut w);
    Some(w.into_inner().into())
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    /// Encodes then decodes `p` and checks equality and that nothing is left.
    pub fn round_trip<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let mut w = Writer::new();
        p.encode(&mut w);
        let b = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&b);
        let back = P::decode_exact(&mut r).unwrap_or_else(|e| panic!("{}: {e}", P::NAME));
        assert_eq!(&back, p, "{}", P::NAME);
    }
}

/// Values of every enum sent as a plain ordinal, pinned to the declaration
/// order in the 26.3 client jar (`cache/mojang-spike/client-26.3.jar`, read
/// with `javap` on the enum's public constants; the audit table is in
/// NOTES.md). A change in any of these silently breaks gameplay, so each
/// test names the jar class it pins.
#[cfg(test)]
mod enum_order_tests {
    use super::configuration::serverbound as cfg_sb;
    use super::play::clientbound as cb;
    use super::play::serverbound as sb;

    /// `net.minecraft.network.protocol.game.ServerboundPlayerActionPacket$Action`
    /// (26.3 inserted CHANGE_DESTROY_DIRECTION at 1, moving every later value).
    #[test]
    fn player_action_status() {
        use sb::player_action_status as a;
        assert_eq!(a::START_DESTROY_BLOCK, 0);
        assert_eq!(a::CHANGE_DESTROY_DIRECTION, 1);
        assert_eq!(a::ABORT_DESTROY_BLOCK, 2);
        assert_eq!(a::STOP_DESTROY_BLOCK, 3);
        assert_eq!(a::DROP_ALL_ITEMS, 4);
        assert_eq!(a::DROP_ITEM, 5);
        assert_eq!(a::RELEASE_USE_ITEM, 6);
        assert_eq!(a::SWAP_ITEM_WITH_OFFHAND, 7);
        assert_eq!(a::STAB, 8);
    }

    /// `net.minecraft.network.protocol.game.ServerboundPlayerCommandPacket$Action`
    /// (STOP_SLEEPING=0, START_SPRINTING=1, STOP_SPRINTING=2,
    /// START_RIDING_JUMP=3, STOP_RIDING_JUMP=4, OPEN_INVENTORY=5,
    /// START_FALL_FLYING=6), documented on the packet.
    #[test]
    fn player_command_action() {
        let p = sb::PlayerCommand {
            entity_id: 1,
            action: 6,
            jump_boost: 0,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.network.protocol.game.ServerboundClientCommandPacket$Action`
    /// (PERFORM_RESPAWN=0, REQUEST_STATS=1, REQUEST_GAMERULE_VALUES=2).
    #[test]
    fn client_command_action() {
        let p = sb::ClientCommand { action: 2 };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.InteractionHand` (MAIN_HAND=0, OFF_HAND=1), the
    /// `hand` field of `interact`, `use_item_on` and `use_item`.
    #[test]
    fn interaction_hand() {
        let p = sb::UseItem {
            hand: 1,
            sequence: 8,
            yaw: 0.0,
            pitch: 0.0,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.entity.player.ChatVisiblity` (FULL=0, SYSTEM=1,
    /// HIDDEN=2), `net.minecraft.world.entity.HumanoidArm` (LEFT=0, RIGHT=1)
    /// and `net.minecraft.server.level.ParticleStatus` (ALL=0, DECREASED=1,
    /// MINIMAL=2) in `client_information`.
    #[test]
    fn client_information_enums() {
        let d = cfg_sb::ClientInformation::defaults("en_us", 8);
        assert_eq!(d.chat_mode, 0);
        assert_eq!(d.main_hand, 1);
        assert_eq!(d.particle_status, 0);
    }

    /// `net.minecraft.network.protocol.common.ServerboundResourcePackPacket$Action`
    /// (SUCCESSFULLY_LOADED=0 .. DISCARDED=7).
    #[test]
    fn resource_pack_status() {
        use crate::connection::resource_pack_status as s;
        assert_eq!(s::SUCCESSFULLY_LOADED, 0);
        assert_eq!(s::DECLINED, 1);
        assert_eq!(s::FAILED_DOWNLOAD, 2);
        assert_eq!(s::ACCEPTED, 3);
        assert_eq!(s::DOWNLOADED, 4);
        assert_eq!(s::INVALID_URL, 5);
        assert_eq!(s::FAILED_RELOAD, 6);
        assert_eq!(s::DISCARDED, 7);
    }

    /// `net.minecraft.world.level.GameType` (SURVIVAL=0, CREATIVE=1,
    /// ADVENTURE=2, SPECTATOR=3), sent as `id + 1` with 0 for none.
    #[test]
    fn game_mode() {
        use haste_core::game::GameMode;
        assert_eq!(GameMode::from_id(0), Some(GameMode::Survival));
        assert_eq!(GameMode::from_id(1), Some(GameMode::Creative));
        assert_eq!(GameMode::from_id(2), Some(GameMode::Adventure));
        assert_eq!(GameMode::from_id(3), Some(GameMode::Spectator));
        assert_eq!(GameMode::from_id(4), None);
    }

    /// `net.minecraft.world.entity.Pose` (STANDING=0, FALL_FLYING=1,
    /// SLEEPING=2, SWIMMING=3, SPIN_ATTACK=4, CROUCHING=5, LONG_JUMPING=6,
    /// DYING=7, CROAKING=8, USING_TONGUE=9, SITTING=10, ROARING=11,
    /// SNIFFING=12, EMERGING=13, DIGGING=14, SLIDING=15, SHOOTING=16,
    /// INHALING=17) — the value of metadata serializer 20.
    #[test]
    fn pose() {
        use haste_core::entity::Pose;
        assert_eq!(Pose::from_id(0), Some(Pose::Standing));
        assert_eq!(Pose::from_id(1), Some(Pose::FallFlying));
        assert_eq!(Pose::from_id(5), Some(Pose::Crouching));
        assert_eq!(Pose::from_id(14), Some(Pose::Digging));
        assert_eq!(Pose::from_id(17), Some(Pose::Inhaling));
        assert_eq!(Pose::from_id(18), None);
    }

    /// `net.minecraft.world.Difficulty` (PEACEFUL=0, EASY=1, NORMAL=2, HARD=3).
    #[test]
    fn difficulty() {
        let p = sb::ChangeDifficulty { difficulty: 3 };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.inventory.RecipeBookType` (CRAFTING=0, FURNACE=1,
    /// BLAST_FURNACE=2, SMOKER=3), the `book` field.
    #[test]
    fn recipe_book_type() {
        let p = sb::RecipeBookChangeSettings {
            book: 2,
            open: true,
            filter: false,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.entity.EquipmentSlot` ordinals, sent in the byte
    /// of each `set_equipment` entry (MAINHAND=0, OFFHAND=1, FEET=2, LEGS=3,
    /// CHEST=4, HEAD=5, BODY=6, SADDLE=7).
    #[test]
    fn equipment_slot() {
        let p = cb::SetEquipment {
            entity_id: 1,
            equipment: vec![(5, Default::default())],
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.entity.Relative`, the `player_position` flag bits
    /// (X=0x01, Y=0x02, Z=0x04, Y_ROT=0x08, X_ROT=0x10, DELTA_X=0x20,
    /// DELTA_Y=0x40, DELTA_Z=0x80, ROTATE_DELTA=0x100).
    #[test]
    fn teleport_flags() {
        use cb::teleport_flags as f;
        assert_eq!(f::X, 0x01);
        assert_eq!(f::Y, 0x02);
        assert_eq!(f::Z, 0x04);
        assert_eq!(f::YAW, 0x08);
        assert_eq!(f::PITCH, 0x10);
        assert_eq!(f::VEL_X, 0x20);
        assert_eq!(f::VEL_Y, 0x40);
        assert_eq!(f::VEL_Z, 0x80);
        assert_eq!(f::ROTATE_VELOCITY, 0x100);
    }

    /// `ClientboundPlayerInfoUpdatePacket$Action` bits in declaration order
    /// (ADD_PLAYER=0x01, INITIALIZE_CHAT=0x02, UPDATE_GAME_MODE=0x04,
    /// UPDATE_LISTED=0x08, UPDATE_LATENCY=0x10, UPDATE_DISPLAY_NAME=0x20,
    /// UPDATE_LIST_ORDER=0x40, UPDATE_HAT=0x80).
    #[test]
    fn player_info_actions() {
        use cb::player_info_actions as a;
        assert_eq!(a::ADD_PLAYER, 0x01);
        assert_eq!(a::INITIALIZE_CHAT, 0x02);
        assert_eq!(a::UPDATE_GAME_MODE, 0x04);
        assert_eq!(a::UPDATE_LISTED, 0x08);
        assert_eq!(a::UPDATE_LATENCY, 0x10);
        assert_eq!(a::UPDATE_DISPLAY_NAME, 0x20);
        assert_eq!(a::UPDATE_LIST_PRIORITY, 0x40);
        assert_eq!(a::UPDATE_HAT, 0x80);
    }

    /// `ClientboundBossEventPacket$OperationType` (ADD=0, REMOVE=1,
    /// UPDATE_PROGRESS=2, UPDATE_NAME=3, UPDATE_STYLE=4, UPDATE_PROPERTIES=5).
    /// The style fields carry `BossEvent$BossBarColor` (PINK=0 .. WHITE=6) and
    /// `BossEvent$BossBarOverlay` (PROGRESS=0, NOTCHED_6=1, NOTCHED_10=2,
    /// NOTCHED_12=3, NOTCHED_20=4); Haste keeps both raw.
    #[test]
    fn boss_event_action() {
        let p = cb::BossEvent {
            id: uuid::Uuid::from_u128(1),
            action: cb::BossEventAction::UpdateStyle {
                color: 6,
                division: 4,
            },
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.scores.DisplaySlot` (LIST=0, SIDEBAR=1,
    /// BELOW_NAME=2, team colour sidebars 3..=18).
    #[test]
    fn display_slot() {
        let p = cb::SetDisplayObjective {
            position: 18,
            name: "n".into(),
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `ClientboundAnimatePacket` action ids (WAKE_UP=0, CRITICAL_HIT=1,
    /// MAGIC_CRITICAL_HIT=2).
    #[test]
    fn animate() {
        let p = cb::Animate {
            entity_id: 1,
            animation: 2,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `ClientboundGameEventPacket$Type` ids (NO_RESPAWN_BLOCK_AVAILABLE=0,
    /// START_RAINING=1, STOP_RAINING=2, CHANGE_GAME_MODE=3, WIN_GAME=4,
    /// DEMO_EVENT=5, PLAY_ARROW_HIT_SOUND=6, RAIN_LEVEL_CHANGE=7,
    /// THUNDER_LEVEL_CHANGE=8, PUFFER_FISH_STING=9, GUARDIAN_ELDER_EFFECT=10,
    /// IMMEDIATE_RESPAWN=11, LIMITED_CRAFTING=12, LEVEL_CHUNKS_LOAD_START=13).
    #[test]
    fn game_event_type() {
        let p = cb::GameEvent {
            event: 13,
            value: 0.0,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.item.SwingAnimationType` (NONE=0, WHACK=1,
    /// STAB=2) in `swing_animation`, with `InteractionHand` for the hand.
    #[test]
    fn swing_animation() {
        let p = cb::SwingAnimation {
            entity_id: 1,
            hand: 0,
            animation: 2,
            duration: 6,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.entity.player.Input` key bits in `player_input`
    /// (FORWARD=0x01, BACKWARD=0x02, LEFT=0x04, RIGHT=0x08, JUMP=0x10,
    /// SNEAK=0x20, SPRINT=0x40).
    #[test]
    fn input_flags() {
        use sb::input_flags as f;
        assert_eq!(f::FORWARD, 0x01);
        assert_eq!(f::BACKWARD, 0x02);
        assert_eq!(f::LEFT, 0x04);
        assert_eq!(f::RIGHT, 0x08);
        assert_eq!(f::JUMP, 0x10);
        assert_eq!(f::SNEAK, 0x20);
        assert_eq!(f::SPRINT, 0x40);
    }

    /// `ServerboundMovePlayerPacket` flag bits (FLAG_ON_GROUND=1,
    /// FLAG_HORIZONTAL_COLLISION=2).
    #[test]
    fn move_flags() {
        use sb::move_flags as f;
        assert_eq!(f::ON_GROUND, 0x01);
        assert_eq!(f::HORIZONTAL_COLLISION, 0x02);
    }

    /// `ServerboundPlayerAbilitiesPacket` flag bits (INVULNERABLE=0x01,
    /// FLYING=0x02, CAN_FLY=0x04, INSTABUILD=0x08).
    #[test]
    fn ability_flags() {
        let p = sb::PlayerAbilities { flags: 0x0e };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.entity.PositionPath$Type` (LINEAR=0, STEPPED=1).
    #[test]
    fn position_path_type() {
        let p = cb::EntityPositionSync {
            entity_id: 1,
            path: cb::PositionPath::Linear(haste_core::DVec3::ZERO),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// Entity metadata serializer type ids, pinned to the registration order
    /// of `EntityDataSerializers.registerSerializer` (0=BYTE .. 43=DYE_COLOR;
    /// 13 is OPTIONAL_LIVING_ENTITY_REFERENCE, an optional UUID).
    #[test]
    fn metadata_serializer_ids() {
        use crate::metadata::types as t;
        assert_eq!(t::BYTE, 0);
        assert_eq!(t::ITEM_STACK, 7);
        assert_eq!(t::BOOLEAN, 8);
        assert_eq!(t::DIRECTION, 12);
        assert_eq!(t::POSE, 20);
        assert_eq!(t::FIRST_VARIANT, 21);
        assert_eq!(t::LAST_VARIANT, 32);
        assert_eq!(t::OPTIONAL_GLOBAL_POS, 33);
        assert_eq!(t::PAINTING_VARIANT, 34);
        assert_eq!(t::VECTOR3, 39);
        assert_eq!(t::QUATERNION, 40);
        assert_eq!(t::HUMANOID_ARM, 42);
        assert_eq!(t::DYE_COLOR, 43);
    }

    /// `net.minecraft.world.level.block.entity.StructureBlockEntity$UpdateType`
    /// (UPDATE_DATA=0, SAVE_AREA=1, LOAD_AREA=2, SCAN_AREA=3) and
    /// `net.minecraft.world.level.block.state.properties.StructureMode`
    /// (SAVE=0, LOAD=1, CORNER=2, DATA=3), `Mirror` (NONE=0, LEFT_RIGHT=1,
    /// FRONT_BACK=2) and `Rotation` (NONE=0, CLOCKWISE_90=1, CLOCKWISE_180=2,
    /// COUNTERCLOCKWISE_90=3) in `set_structure_block`.
    #[test]
    fn structure_block_enums() {
        let p = sb::SetStructureBlock {
            pos: haste_core::BlockPos::new(0, 0, 0),
            action: 2,
            mode: 3,
            name: "s".into(),
            offset_x: 0,
            offset_y: 1,
            offset_z: -1,
            size_x: 5,
            size_y: 5,
            size_z: 5,
            mirror: 2,
            rotation: 3,
            metadata: String::new(),
            integrity: 1.0,
            seed: 0,
            flags: 0x0f,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.level.block.state.properties.TestBlockMode`
    /// (START=0, LOG=1, FAIL=2, ACCEPT=3).
    #[test]
    fn test_block_mode() {
        let p = sb::SetTestBlock {
            pos: haste_core::BlockPos::new(0, 0, 0),
            mode: 3,
            message: "m".into(),
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.level.block.entity.CommandBlockEntity$Mode`
    /// (SEQUENCE=0, AUTO=1, REDSTONE=2; chain, repeating, impulse).
    #[test]
    fn command_block_mode() {
        let p = sb::SetCommandBlock {
            pos: haste_core::BlockPos::new(0, 0, 0),
            command: "say hi".into(),
            mode: 1,
            flags: 7,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.level.block.entity.SignTextSlot` (BACK=0,
    /// FRONT=1).
    #[test]
    fn sign_text_slot() {
        let p = sb::SignUpdate {
            pos: haste_core::BlockPos::new(1, 2, 3),
            line1: "a".into(),
            line2: String::new(),
            line3: String::new(),
            line4: "d".into(),
            text_slot: 1,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.inventory.ContainerInput` (PICKUP=0,
    /// QUICK_MOVE=1, SWAP=2, CLONE=3, THROW=4, QUICK_CRAFT=5, PICKUP_ALL=6):
    /// Haste keeps `container_click`'s mode raw, so this only pins the
    /// round-trip of a non-zero mode.
    #[test]
    fn container_click_mode() {
        let p = sb::ContainerClick {
            window_id: 0,
            state_id: 1,
            slot: 9,
            button: 0,
            mode: 5,
            changed: vec![],
            carried: Default::default(),
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.network.chat.numbers.NumberFormatTypes` registry order
    /// (blank=0, styled=1, fixed=2), sent in `set_objective` display data.
    #[test]
    fn number_format() {
        use cb::NumberFormat as n;
        let p = n::Fixed(haste_core::Component::text("x"));
        let mut w = crate::wire::Writer::new();
        crate::wire::Wire::write(&p, &mut w);
        // id 2, then the component's bytes.
        assert_eq!(w.buf[0], 2);
    }

    /// `net.minecraft.world.scores.criteria.ObjectiveCriteria$RenderType`
    /// (INTEGER=0, HEARTS=1).
    #[test]
    fn objective_render_type() {
        let p = cb::SetObjective {
            name: "n".into(),
            mode: 0,
            display: Some(cb::ObjectiveDisplay {
                value: haste_core::Component::text("v"),
                render_type: 1,
                number_format: None,
            }),
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.scores.Team$Visibility` (ALWAYS=0, NEVER=1,
    /// HIDE_FOR_OTHER_TEAMS=2, HIDE_FOR_OWN_TEAM=3) and `Team$CollisionRule`
    /// (ALWAYS=0, NEVER=1, PUSH_OTHER_TEAMS=2, PUSH_OWN_TEAM=3).
    #[test]
    fn team_visibility_and_collision() {
        let p = cb::SetPlayerTeam {
            name: "t".into(),
            method: 0,
            parameters: Some(cb::TeamParameters {
                display_name: haste_core::Component::text("d"),
                prefix: haste_core::Component::text(""),
                suffix: haste_core::Component::text(""),
                name_tag_visibility: 3,
                collision_rule: 2,
                color: Some(14),
                flags: 0,
            }),
            entities: vec!["player".into()],
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.sounds.SoundSource` (MASTER=0, MUSIC=1, RECORDS=2,
    /// WEATHER=3, BLOCKS=4, HOSTILE=5, NEUTRAL=6, PLAYERS=7, AMBIENT=8,
    /// VOICE=9, UI=10) — the `stop_sound` category is kept raw in Haste.
    #[test]
    fn sound_source() {
        let p = cb::StopSound {
            source: Some(10),
            sound: None,
        };
        crate::packets::test_util::round_trip(&p);
    }

    /// `net.minecraft.world.item.consume_effects.ConsumeEffect$Type`
    /// registry order (apply_effects=0, remove_effects=1, clear_all_effects=2,
    /// teleport_randomly=3, play_sound=4), skipped when decoding item
    /// components; the skip shapes match in `wire::slot`.
    #[test]
    fn consume_effect_type_ids() {
        let by_id = [
            "apply_effects",
            "remove_effects",
            "clear_all_effects",
            "teleport_randomly",
            "play_sound",
        ];
        for (i, name) in by_id.iter().enumerate() {
            assert_eq!(i as i32, consume_effect_id(name), "{name}");
        }
        fn consume_effect_id(name: &str) -> i32 {
            match name {
                "apply_effects" => 0,
                "remove_effects" => 1,
                "clear_all_effects" => 2,
                "teleport_randomly" => 3,
                "play_sound" => 4,
                _ => unreachable!(),
            }
        }
    }
}
