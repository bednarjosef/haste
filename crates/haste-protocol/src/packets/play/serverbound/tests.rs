use super::*;

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&BlockEntityTagQuery {
            transaction_id: 1,
            pos: BlockPos::new(1, 2, 3),
        });
        round_trip(&BundleItemSelected {
            slot: 3,
            slot_in_bundle: -1,
        });
        round_trip(&ChangeDifficulty { difficulty: 2 });
        round_trip(&ChangeGameMode { game_mode: 1 });
        round_trip(&ChatCommandSigned {
            command: "msg a hi".into(),
            timestamp: 1,
            salt: 2,
            arguments: vec![ArgumentSignature {
                name: "message".into(),
                signature: Bytes::from(vec![3; 256]),
            }],
            message_count: 0,
            acknowledged: [0; 3],
            checksum: 0,
        });
        round_trip(&ChatSessionUpdate {
            session: ChatSession {
                session_id: Uuid::from_u128(1),
                expires_at: 2,
                public_key: Bytes::from_static(&[1, 2]),
                key_signature: Bytes::from_static(&[3]),
            },
        });
        round_trip(&ClientCommand { action: 0 });
        round_trip(&CommandSuggestion {
            id: 1,
            text: "/tp ".into(),
        });
        round_trip(&ContainerButtonClick {
            window_id: 1,
            button: 2,
        });
        round_trip(&ContainerClick {
            window_id: 0,
            state_id: 5,
            slot: 36,
            button: 0,
            mode: 0,
            changed: vec![ChangedSlot {
                slot: 36,
                item: HashedSlot {
                    item: Some(HashedItem {
                        item: 1,
                        count: 64,
                        added: vec![(3, 12345)],
                        removed: vec![4],
                    }),
                },
            }],
            carried: HashedSlot::default(),
        });
        round_trip(&ContainerClose { window_id: 0 });
        round_trip(&ContainerSlotStateChanged {
            slot: 1,
            window_id: 2,
            enabled: true,
        });
        round_trip(&DebugSubscriptionRequest {
            subscriptions: vec![0, 3],
        });
        round_trip(&EditBook {
            slot: 0,
            pages: vec!["p1".into()],
            title: Some("t".into()),
        });
        round_trip(&EntityTagQuery {
            transaction_id: 1,
            entity_id: 2,
        });
        round_trip(&JigsawGenerate {
            pos: BlockPos::new(0, 0, 0),
            levels: 3,
            keep_jigsaws: true,
        });
        round_trip(&LockDifficulty { locked: true });
        round_trip(&MoveVehicle {
            pos: DVec3::ONE,
            yaw: 1.0,
            pitch: 2.0,
            on_ground: false,
        });
        round_trip(&PaddleBoat {
            left: true,
            right: false,
        });
        round_trip(&PickItemFromBlock {
            pos: BlockPos::new(1, 1, 1),
            include_data: false,
        });
        round_trip(&PickItemFromEntity {
            entity_id: 5,
            include_data: true,
        });
        round_trip(&PingRequest { payload: 9 });
        round_trip(&PlaceRecipe {
            window_id: 1,
            recipe: 2,
            make_all: false,
        });
        round_trip(&PlayerAbilities { flags: 2 });
        round_trip(&RecipeBookChangeSettings {
            book: 0,
            open: true,
            filter: false,
        });
        round_trip(&RecipeBookSeenRecipe { recipe: 3 });
        round_trip(&RenameItem { name: "x".into() });
        round_trip(&SeenAdvancements {
            action: 0,
            tab: Some(Identifier::minecraft("story/root")),
        });
        round_trip(&SeenAdvancements {
            action: 1,
            tab: None,
        });
        round_trip(&SelectTrade { slot: 2 });
        round_trip(&SetBeacon {
            primary: Some(1),
            secondary: None,
        });
        round_trip(&SetCommandBlock {
            pos: BlockPos::new(0, 0, 0),
            command: "say hi".into(),
            mode: 2,
            flags: 1,
        });
        round_trip(&SetCommandMinecart {
            entity_id: 1,
            command: "say".into(),
            track_output: true,
        });
        round_trip(&SetCreativeModeSlot {
            slot: 36,
            item: UntrustedItemStack {
                count: 1,
                item: 5,
                added: vec![(3, Bytes::from_static(&[7]))],
                removed: vec![2],
            },
        });
        round_trip(&SetCreativeModeSlot {
            slot: -1,
            item: UntrustedItemStack::default(),
        });
        round_trip(&SetGameRule {
            rules: vec![GameRuleValue {
                name: Identifier::minecraft("keep_inventory"),
                value: "true".into(),
            }],
        });
        round_trip(&SetJigsawBlock {
            pos: BlockPos::new(1, 2, 3),
            name: Identifier::minecraft("a"),
            target: Identifier::minecraft("b"),
            pool: Identifier::minecraft("c"),
            final_state: "minecraft:air".into(),
            joint_type: "rollable".into(),
            selection_priority: 0,
            placement_priority: 0,
        });
        round_trip(&SetStructureBlock {
            pos: BlockPos::new(0, 0, 0),
            action: 0,
            mode: 0,
            name: "s".into(),
            offset_x: 0,
            offset_y: 1,
            offset_z: -1,
            size_x: 5,
            size_y: 5,
            size_z: 5,
            mirror: 0,
            rotation: 0,
            metadata: String::new(),
            integrity: 1.0,
            seed: 0,
            flags: 4,
        });
        round_trip(&SetTestBlock {
            pos: BlockPos::new(0, 0, 0),
            mode: 1,
            message: "m".into(),
        });
        round_trip(&SignUpdate {
            pos: BlockPos::new(1, 2, 3),
            line1: "a".into(),
            line2: String::new(),
            line3: String::new(),
            line4: "d".into(),
            text_slot: 1,
        });
        round_trip(&SpectatorAction { entity: 0 });
        round_trip(&TeleportToEntity {
            target: Uuid::from_u128(4),
        });
        round_trip(&TestInstanceBlockAction {
            body: Bytes::from_static(&[1, 2]),
        });
    }
}

#[cfg(test)]
mod packet_tests {
    use super::*;
    use crate::packets::test_util::round_trip;

    #[test]
    fn round_trips() {
        round_trip(&AcceptTeleportation {
            teleport_id: 1,
            pos: DVec3::new(0.5, 64.0, 0.5),
            yaw: 90.0,
            pitch: 0.0,
        });
        round_trip(&Attack { entity_id: 5 });
        round_trip(&ChatAck { message_count: 2 });
        round_trip(&ChatCommand {
            command: "time set day".into(),
        });
        round_trip(&Chat {
            message: "hi".into(),
            timestamp: 1,
            salt: 2,
            signature: None,
            message_count: 0,
            acknowledged: [0; 3],
            checksum: 0,
        });
        round_trip(&Chat {
            message: "hi".into(),
            timestamp: 1,
            salt: 2,
            signature: Some(Bytes::from(vec![1; 256])),
            message_count: 1,
            acknowledged: [1, 0, 0],
            checksum: 5,
        });
        round_trip(&ChunkBatchReceived {
            chunks_per_tick: 64.0,
        });
        round_trip(&ClientTickEnd {});
        round_trip(&ClientInformation::defaults("en_us", 8));
        round_trip(&ConfigurationAcknowledged {});
        round_trip(&Interact {
            entity_id: 1,
            hand: 0,
            target_offset: DVec3::ZERO,
            sneaking: false,
        });
        round_trip(&MovePlayerPos {
            pos: DVec3::new(1.0, 2.0, 3.0),
            flags: move_flags::ON_GROUND,
        });
        round_trip(&MovePlayerPosRot {
            pos: DVec3::ZERO,
            yaw: 1.0,
            pitch: 2.0,
            flags: 0,
        });
        round_trip(&MovePlayerRot {
            yaw: 1.0,
            pitch: 2.0,
            flags: 3,
        });
        round_trip(&MovePlayerStatusOnly { flags: 1 });
        round_trip(&PlayerAction {
            status: 0,
            pos: BlockPos::new(1, 2, 3),
            face: 1,
            sequence: 4,
        });
        round_trip(&PlayerCommand {
            entity_id: 1,
            action: 1,
            jump_boost: 0,
        });
        round_trip(&PlayerInput {
            flags: input_flags::FORWARD | input_flags::SPRINT,
        });
        round_trip(&PlayerLoaded {});
        round_trip(&Punch {});
        round_trip(&SetCarriedItem { slot: 3 });
        round_trip(&UseItemOn {
            hand: 0,
            pos: BlockPos::new(0, 64, 0),
            face: 1,
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: 7,
        });
        round_trip(&UseItem {
            hand: 1,
            sequence: 8,
            yaw: 0.0,
            pitch: 0.0,
        });
    }
}
