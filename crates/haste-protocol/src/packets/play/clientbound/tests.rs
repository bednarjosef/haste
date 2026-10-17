use super::*;

#[cfg(test)]
mod more_tests {
    use super::*;
    use crate::packets::test_util::round_trip;
    use crate::packets::ProtocolTables;

    fn tables() -> ProtocolTables {
        ProtocolTables::for_tests(&[(3, "minecraft:damage")]).with_particles_for_tests(&[
            (0, "minecraft:explosion"),
            (1, "minecraft:block"),
            (2, "minecraft:dust"),
        ])
    }

    fn round_trip_tables<P: Packet + PartialEq + std::fmt::Debug>(p: &P) {
        let t = tables();
        let mut w = Writer::new();
        p.encode(&mut w);
        let b = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&b).with_tables(&t);
        let back = P::decode_exact(&mut r).unwrap_or_else(|e| panic!("{}: {e}", P::NAME));
        assert_eq!(&back, p, "{}", P::NAME);
    }

    fn item() -> ItemStackData {
        ItemStackData {
            count: 1,
            item: Some(RegistryIndex(7)),
            components_raw: Bytes::from_static(&[0, 0]),
        }
    }
    fn nbt() -> Bytes {
        let mut c = crate::nbt::NbtCompound::new();
        c.put_string("color", "red".into());
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        w.into_inner().into()
    }

    #[test]
    fn round_trips() {
        round_trip(&Animate {
            entity_id: 1,
            animation: 2,
        });
        round_trip(&AwardStats {
            stats: vec![Statistic {
                category: 8,
                stat: 3,
                value: 100,
            }],
        });
        round_trip(&BlockChangedAck { sequence: 5 });
        round_trip(&BlockDestruction {
            entity_id: 1,
            pos: BlockPos::new(1, 2, 3),
            stage: 4,
        });
        round_trip(&BlockEvent {
            pos: BlockPos::new(0, 1, 0),
            action: 1,
            param: 1,
            block: 200,
        });
        let id = Uuid::from_u128(1);
        for action in [
            BossEventAction::Add {
                title: Component::text("Wither"),
                health: 1.0,
                color: 5,
                division: 0,
                flags: 1,
            },
            BossEventAction::Remove,
            BossEventAction::UpdateHealth(0.5),
            BossEventAction::UpdateTitle(Component::text("t")),
            BossEventAction::UpdateStyle {
                color: 1,
                division: 2,
            },
            BossEventAction::UpdateFlags(4),
        ] {
            round_trip(&BossEvent { id, action });
        }
        round_trip(&ChunksBiomes {
            chunks: vec![ChunkBiomes {
                pos: ChunkPos::new(-1, 3),
                data: Bytes::from_static(&[0, 1, 2]),
            }],
        });
        round_trip(&ClearTitles { reset: true });
        round_trip(&CommandSuggestions {
            id: 1,
            start: 2,
            length: 3,
            matches: vec![SuggestionMatch {
                text: "a".into(),
                tooltip: Some(Component::text("t")),
            }],
        });
        round_trip(&ContainerClose { window_id: 3 });
        round_trip(&ContainerSetData {
            window_id: 1,
            property: 2,
            value: -1,
        });
        round_trip(&Cooldown {
            group: Identifier::minecraft("ender_pearl"),
            ticks: 20,
        });
        round_trip(&CustomChatCompletions {
            action: 2,
            entries: vec!["x".into()],
        });
        round_trip(&DamageEvent {
            entity_id: 1,
            source_type: 2,
            source_cause: 0,
            source_direct: 0,
            source_pos: Some(DVec3::ONE),
        });
        round_trip(&DebugBlockValue {
            pos: BlockPos::new(1, 1, 1),
            update: Bytes::from_static(&[0, 0]),
        });
        round_trip(&DebugChunkValue {
            pos: ChunkPos::new(1, 2),
            update: Bytes::from_static(&[1]),
        });
        round_trip(&DebugEntityValue {
            entity_id: 3,
            update: Bytes::new(),
        });
        round_trip(&DebugEvent {
            event: Bytes::from_static(&[0]),
        });
        round_trip(&DebugSample {
            sample: vec![1, 2, 3, 4],
            sample_type: 0,
        });
        round_trip(&DeleteChat {
            message: PreviousMessage::Id(4),
        });
        round_trip(&DeleteChat {
            message: PreviousMessage::Signature(Bytes::from(vec![2; 256])),
        });
        round_trip(&EntityEvent {
            entity_id: 5,
            status: 9,
        });
        round_trip_tables(&Explode {
            center: DVec3::new(1.0, 2.0, 3.0),
            radius: 4.0,
            block_count: 10,
            player_knockback: Some(DVec3::X),
            particle: Particle {
                kind: 0,
                data: Bytes::new(),
            },
            sound: IdOr::Id(5),
            block_particles: vec![ExplosionParticle {
                particle: Particle {
                    kind: 1,
                    data: Bytes::from_static(&[9]),
                },
                scaling: 1.0,
                speed: 0.5,
                weight: 3,
            }],
            play_sound: true,
        });
        round_trip(&AddTransientBlock {
            pos: BlockPos::new(0, 0, 0),
            state: 1,
        });
        round_trip(&GameRuleValues {
            rules: vec![GameRuleValue {
                name: Identifier::minecraft("keep_inventory"),
                value: "true".into(),
            }],
        });
        round_trip(&GameTestHighlightPos {
            absolute: BlockPos::new(1, 2, 3),
            relative: BlockPos::new(0, 0, 1),
        });
        round_trip(&MountScreenOpen {
            window_id: 1,
            columns: 5,
            entity_id: 9,
        });
        round_trip(&HurtAnimation {
            entity_id: 1,
            yaw: 90.0,
        });
        round_trip(&InitializeBorder {
            x: 0.0,
            z: 0.0,
            old_diameter: 1e7,
            new_diameter: 1e7,
            speed: 0,
            portal_boundary: 29999984,
            warning_blocks: 5,
            warning_time: 15,
        });
        round_trip_tables(&LevelParticles {
            particle: Particle {
                kind: 2,
                data: Bytes::from_static(&[0, 0, 0xff, 0, 0x3f, 0x80, 0, 0]),
            },
            override_limiter: false,
            always_visible: true,
            pos: DVec3::ONE,
            offset_x: 0.1,
            offset_y: 0.2,
            offset_z: 0.3,
            max_speed_x: 1.0,
            max_speed_y: 1.0,
            max_speed_z: 1.0,
            count: 10,
            randomization: 0,
        });
        round_trip(&LowDiskSpaceWarning {});
        round_trip(&MapItemData {
            map_id: 1,
            scale: 0,
            locked: false,
            icons: Some(vec![MapIcon {
                kind: 0,
                x: 1,
                z: -1,
                direction: 3,
                name: None,
            }]),
            patch: Some(MapPatch {
                columns: 1,
                rows: 2,
                x: 0,
                z: 0,
                data: Bytes::from_static(&[4, 5]),
            }),
        });
        round_trip(&MapItemData {
            map_id: 1,
            scale: 4,
            locked: true,
            icons: None,
            patch: None,
        });
        round_trip_tables(&MerchantOffers {
            window_id: 1,
            offers: vec![MerchantOffer {
                input1: TradeItem {
                    item: 1,
                    count: 12,
                    components_raw: Bytes::from_static(&[1, 3, 5]),
                },
                output: item(),
                input2: None,
                disabled: false,
                uses: 0,
                max_uses: 12,
                xp: 1,
                special_price: 0,
                price_multiplier: 0.05,
                demand: 0,
            }],
            level: 1,
            experience: 0,
            regular_villager: true,
            can_restock: true,
        });
        round_trip(&MoveMinecartAlongTrack {
            entity_id: 1,
            steps: vec![MinecartStep {
                pos: DVec3::ONE,
                velocity: DVec3::ZERO,
                yaw: 90.0,
                pitch: 0.0,
                weight: 1.0,
            }],
        });
        round_trip(&MoveVehicle {
            pos: DVec3::ONE,
            yaw: 1.0,
            pitch: 2.0,
        });
        round_trip(&OpenBook { hand: 0 });
        round_trip(&OpenScreen {
            window_id: 1,
            kind: 2,
            title: Component::text("Chest"),
        });
        round_trip(&OpenSignEditor {
            pos: BlockPos::new(1, 2, 3),
            text_slot: 1,
        });
        round_trip(&PongResponse { payload: 77 });
        round_trip(&PlaceGhostRecipe {
            window_id: 1,
            display: Bytes::from_static(&[3, 0]),
        });
        round_trip(&PlayerAbilities {
            flags: 0x0d,
            flying_speed: 0.05,
            fov_modifier: 0.1,
        });
        round_trip(&PlayerCombatEnd { duration: 5 });
        round_trip(&PlayerCombatEnter {});
        round_trip(&PlayerCombatKill {
            player_id: 1,
            message: Component::text("died"),
        });
        round_trip(&PlayerLookAt {
            from_anchor: 1,
            target: DVec3::ONE,
            entity: Some(LookAtEntity {
                entity_id: 3,
                anchor: 0,
            }),
        });
        round_trip(&PlayerLookAt {
            from_anchor: 0,
            target: DVec3::ZERO,
            entity: None,
        });
        round_trip(&PlayerRotation {
            yaw: 10.0,
            relative_yaw: true,
            pitch: -5.0,
            relative_pitch: false,
        });
        round_trip(&RemoveMobEffect {
            entity_id: 1,
            effect: 2,
        });
        round_trip(&ResetScore {
            owner: "Steve".into(),
            objective: Some("kills".into()),
        });
        round_trip(&SelectAdvancementsTab {
            tab: Some(Identifier::minecraft("story/root")),
        });
        round_trip(&SetActionBarText {
            text: Component::text("a"),
        });
        round_trip(&SetBorderCenter { x: 1.0, z: 2.0 });
        round_trip(&SetBorderLerpSize {
            old_diameter: 1.0,
            new_diameter: 2.0,
            speed: 1000,
        });
        round_trip(&SetBorderSize { diameter: 100.0 });
        round_trip(&SetBorderWarningDelay { warning_time: 15 });
        round_trip(&SetBorderWarningDistance { warning_blocks: 5 });
        round_trip(&SetCamera { camera_id: 1 });
        round_trip(&SetCursorItem { item: item() });
        round_trip(&SetDefaultSpawnPosition {
            dimension: Identifier::minecraft("overworld"),
            pos: BlockPos::new(0, 64, 0),
            yaw: 0.0,
            pitch: 0.0,
        });
        round_trip(&SetDisplayObjective {
            position: 1,
            name: "obj".into(),
        });
        round_trip(&SetEntityLink {
            attached: 1,
            holder: -1,
        });
        round_trip(&SetObjective {
            name: "o".into(),
            mode: 0,
            display: Some(ObjectiveDisplay {
                value: Component::text("O"),
                render_type: 1,
                number_format: Some(NumberFormat::Styled(nbt())),
            }),
        });
        round_trip(&SetObjective {
            name: "o".into(),
            mode: 1,
            display: None,
        });
        round_trip(&SetPassengers {
            vehicle: 1,
            passengers: vec![2, 3],
        });
        round_trip(&SetPlayerInventory {
            slot: 40,
            item: item(),
        });
        round_trip(&SetPlayerTeam {
            name: "red".into(),
            method: 0,
            parameters: Some(TeamParameters {
                display_name: Component::text("Red"),
                prefix: Component::default(),
                suffix: Component::default(),
                name_tag_visibility: 0,
                collision_rule: 0,
                color: Some(12),
                flags: 3,
            }),
            entities: vec!["Steve".into()],
        });
        round_trip(&SetPlayerTeam {
            name: "red".into(),
            method: 1,
            parameters: None,
            entities: vec![],
        });
        round_trip(&SetPlayerTeam {
            name: "red".into(),
            method: 4,
            parameters: None,
            entities: vec!["Alex".into()],
        });
        round_trip(&SetScore {
            owner: "Steve".into(),
            objective: "o".into(),
            value: 5,
            display_name: None,
            number_format: Some(NumberFormat::Fixed(Component::text("x"))),
        });
        round_trip(&SetScore {
            owner: "a".into(),
            objective: "b".into(),
            value: -1,
            display_name: Some(Component::text("n")),
            number_format: Some(NumberFormat::Blank),
        });
        round_trip(&SetSubtitleText {
            text: Component::text("s"),
        });
        round_trip(&SetTitleText {
            text: Component::text("t"),
        });
        round_trip(&SetTitlesAnimation {
            fade_in: 10,
            stay: 70,
            fade_out: 20,
        });
        round_trip(&StopSound {
            source: Some(3),
            sound: None,
        });
        round_trip(&StopSound {
            source: None,
            sound: Some(Identifier::minecraft("music.game")),
        });
        round_trip(&StopSound {
            source: Some(0),
            sound: Some(Identifier::minecraft("x")),
        });
        round_trip(&SwingAnimation {
            entity_id: 1,
            hand: 0,
            animation: 1,
            duration: 6,
        });
        round_trip(&TagQuery {
            transaction_id: 1,
            nbt: nbt(),
        });
        round_trip(&TagQuery {
            transaction_id: 1,
            nbt: Bytes::from_static(&[0]),
        });
        round_trip(&TakeItemEntity {
            collected: 1,
            collector: 2,
            count: 3,
        });
        round_trip(&TestInstanceBlockStatus {
            body: Bytes::from_static(&[8, 0, 1, b'a', 0]),
        });
        round_trip(&TickingState {
            tick_rate: 20.0,
            frozen: false,
        });
        round_trip(&TickingStep { steps: 3 });
        round_trip(&UpdateAdvancements {
            reset: true,
            added: vec![AdvancementEntry {
                id: Identifier::minecraft("story/root"),
                advancement: Advancement {
                    parent: None,
                    display: Some(AdvancementDisplay {
                        title: Component::text("Minecraft"),
                        description: Component::text("d"),
                        icon: item(),
                        frame: 0,
                        flags: 1,
                        background: Some(Identifier::minecraft(
                            "gui/advancements/backgrounds/stone",
                        )),
                    }),
                    requirements: vec![vec!["crafting_table".into()]],
                    telemetry: true,
                    x: 0.0,
                    y: 1.5,
                },
            }],
            removed: vec![Identifier::minecraft("old")],
            progress: vec![AdvancementProgress {
                id: Identifier::minecraft("story/root"),
                criteria: vec![CriterionProgress {
                    criterion: "crafting_table".into(),
                    achieved_at: Some(1),
                }],
            }],
            show_advancements: false,
        });
        round_trip(&UpdateAttributes {
            entity_id: 1,
            properties: vec![AttributeProperty {
                id: 3,
                value: 0.1,
                modifiers: vec![AttributeModifier {
                    id: Identifier::minecraft("sprinting"),
                    amount: 0.3,
                    operation: 2,
                }],
            }],
        });
        round_trip(&UpdateMobEffect {
            entity_id: 1,
            effect: 2,
            amplifier: 0,
            duration: -1,
            flags: 6,
        });
        round_trip(&ProjectilePower {
            entity_id: 1,
            power: 0.1,
        });
        round_trip(&Waypoint {
            body: Bytes::from_static(&[0]),
        });
        round_trip(&ShowDialog {
            dialog: IdOr::Id(3),
        });
        round_trip(&ShowDialog {
            dialog: IdOr::Inline(DialogNbt(nbt())),
        });
    }

    /// A body written the way vanilla writes it decodes to the same bytes again: the UI
    /// forwards `update_advancements` into Java in its wire form, so a name written
    /// differently than it was read would hand vanilla a different advancement tree.
    #[test]
    fn update_advancements_keeps_the_servers_bytes() {
        let mut w = Writer::new();
        // should_reset, then two added advancements (a root and its child, both without a
        // display, as a vanilla server sends a recipe-only tree).
        w.bool(true);
        w.var_int(2);
        w.identifier(&Identifier::minecraft("story/root"));
        w.bool(false);
        w.bool(false);
        w.var_int(1);
        w.var_int(1);
        w.string("crafting_table");
        w.bool(false);
        w.f32(0.0);
        w.f32(0.0);
        w.identifier(&Identifier::minecraft("story/mine_stone"));
        w.bool(true);
        w.identifier(&Identifier::minecraft("story/root"));
        w.bool(false);
        w.var_int(1);
        w.var_int(1);
        w.string("crafting_table");
        w.bool(false);
        w.f32(0.0);
        w.f32(0.0);
        // one removed advancement.
        w.var_int(1);
        w.identifier(&Identifier::minecraft("story/gone"));
        // progress for the root: its criteria by their plain names, with the obtained
        // instants (a missing instant is a criterion that is not done).
        w.var_int(1);
        w.identifier(&Identifier::minecraft("story/root"));
        w.var_int(2);
        w.string("crafting_table");
        w.bool(true);
        w.i64(1_770_000_000_000);
        w.string("unlock_right_away");
        w.bool(false);
        w.bool(true);
        let body = Bytes::from(w.into_inner());
        let mut r = Reader::from_bytes(&body);
        let packet = UpdateAdvancements::decode_exact(&mut r)
            .unwrap_or_else(|e| panic!("{}: {e}", UpdateAdvancements::NAME));
        let mut w = Writer::new();
        packet.encode(&mut w);
        assert_eq!(w.into_inner(), body, "re-encoded bytes differ");
        assert_eq!(
            packet.progress[0].criteria[0].criterion, "crafting_table",
            "the criterion names are the plain strings the server used"
        );
    }
}

#[cfg(test)]
mod packet_tests {
    use super::*;
    use crate::packets::test_util::round_trip;
    use haste_core::entity::MetadataValue;

    fn nbt() -> Bytes {
        let mut c = crate::nbt::NbtCompound::new();
        c.put_string("id", "x".into());
        let mut w = Writer::new();
        w.nbt_compound(Some(&c));
        w.into_inner().into()
    }
    fn item() -> ItemStackData {
        ItemStackData {
            count: 2,
            item: Some(RegistryIndex(1)),
            components_raw: Bytes::from_static(&[0, 0]),
        }
    }
    fn chat_type() -> ChatType {
        let d = ChatDecoration {
            translation_key: "chat.type.text".into(),
            parameters: vec![0, 2],
            style: Bytes::from_static(&[0]),
        };
        ChatType {
            chat: d.clone(),
            narration: d,
        }
    }
    fn spawn_info() -> SpawnInfo {
        SpawnInfo {
            dimension_type: 0,
            dimension: Identifier::minecraft("overworld"),
            hashed_seed: -42,
            game_mode: 1,
            previous_game_mode: 0,
            is_debug: false,
            is_flat: true,
            death_location: Some(GlobalPos {
                dimension: Identifier::minecraft("the_nether"),
                pos: BlockPos::new(1, 2, 3),
            }),
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    #[test]
    fn round_trips() {
        round_trip(&BundleDelimiter {});
        round_trip(&AddEntity {
            entity_id: 7,
            uuid: Uuid::from_u128(3),
            kind: 140,
            pos: DVec3::new(1.5, 64.0, -3.25),
            velocity: DVec3::ZERO,
            pitch: 45.0,
            yaw: 90.0,
            head_yaw: 180.0,
            data: 0,
        });
        round_trip(&BlockEntityData {
            pos: BlockPos::new(1, 2, 3),
            kind: 7,
            nbt: nbt(),
        });
        round_trip(&BlockUpdate {
            pos: BlockPos::new(-1, -64, 5),
            state: 1234,
        });
        round_trip(&ChangeDifficulty {
            difficulty: 2,
            locked: false,
        });
        round_trip(&ChunkBatchFinished { batch_size: 25 });
        round_trip(&ChunkBatchStart {});
        round_trip(&Commands {
            body: Bytes::from_static(&[1, 2, 3]),
        });
        round_trip(&ContainerSetContent {
            window_id: 0,
            state_id: 1,
            slots: vec![ItemStackData::default(), item()],
            carried: ItemStackData::default(),
        });
        round_trip(&ContainerSetSlot {
            window_id: 0,
            state_id: 2,
            slot: 36,
            item: item(),
        });
        round_trip(&DisguisedChat {
            message: Component::text("hi"),
            chat_type: IdOr::Inline(chat_type()),
            sender_name: Component::text("Server"),
            target_name: None,
        });
        round_trip(&EntityPositionSync {
            entity_id: 1,
            path: PositionPath::Linear(DVec3::new(1.0, 2.0, 3.0)),
            yaw: 10.0,
            pitch: -5.0,
            on_ground: true,
        });
        round_trip(&EntityPositionSync {
            entity_id: 1,
            path: PositionPath::Stepped(vec![(DVec3::X, 1), (DVec3::Y, 2)]),
            yaw: 0.0,
            pitch: 0.0,
            on_ground: false,
        });
        round_trip(&ForgetLevelChunk {
            pos: ChunkPos::new(-3, 7),
        });
        round_trip(&GameEvent {
            event: 13,
            value: 0.0,
        });
        round_trip(&LevelChunkWithLight {
            chunk: ChunkData {
                pos: ChunkPos::new(-2, 5),
                heightmaps: vec![(1, vec![1, 2, 3]), (4, vec![])],
                sections: Bytes::from_static(&[0, 1, 0, 0]),
                block_entities: vec![ChunkBlockEntity {
                    pos: BlockPos::new(-32 + 3, -60, 80 + 15),
                    kind: 9,
                    nbt: nbt(),
                }],
                light: LightData {
                    sky_mask: vec![3],
                    block_mask: vec![],
                    empty_sky_mask: vec![4],
                    empty_block_mask: vec![],
                    sky_arrays: vec![Bytes::from(vec![0xff; 2048]), Bytes::from(vec![0; 2048])],
                    block_arrays: vec![],
                },
            },
        });
        round_trip(&LevelEvent {
            event: 1000,
            pos: BlockPos::new(0, 0, 0),
            data: 0,
            global: false,
        });
        round_trip(&LightUpdate {
            pos: ChunkPos::new(1, 2),
            light: LightData::default(),
        });
        round_trip(&Login {
            entity_id: 42,
            hardcore: false,
            dimension_names: vec![Identifier::minecraft("overworld")],
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            spawn: spawn_info(),
            online_mode: false,
            enforces_secure_chat: false,
        });
        round_trip(&MoveEntityPos {
            entity_id: 3,
            deltas: MoveDeltas::Linear([4096, -1, 0]),
            on_ground: true,
        });
        round_trip(&MoveEntityPos {
            entity_id: 3,
            deltas: MoveDeltas::Stepped(vec![(0, [1, 2, 3]), (1, [4, 5, 6])]),
            on_ground: false,
        });
        round_trip(&MoveEntityPosRot {
            entity_id: 3,
            deltas: MoveDeltas::Linear([0, 0, 1]),
            on_ground: false,
            yaw: 90.0,
            pitch: 0.0,
        });
        round_trip(&MoveEntityRot {
            entity_id: 3,
            on_ground: true,
            yaw: 45.0,
            pitch: 270.0,
        });
        round_trip(&PlayerChat {
            global_index: 0,
            sender: Uuid::from_u128(1),
            index: 0,
            signature: Some(Bytes::from(vec![7; 256])),
            message: "hello".into(),
            timestamp: 1,
            salt: 2,
            previous: vec![
                PreviousMessage::Id(3),
                PreviousMessage::Signature(Bytes::from(vec![1; 256])),
            ],
            unsigned_content: None,
            filter_type: 2,
            filter_bits: Some(vec![5]),
            chat_type: IdOr::Id(0),
            sender_name: Component::text("Steve"),
            target_name: Some(Component::text("Alex")),
        });
        round_trip(&PlayerInfoRemove {
            uuids: vec![Uuid::from_u128(9)],
        });
        round_trip(&PlayerInfoUpdate {
            actions: 0xff,
            entries: vec![PlayerInfoEntry {
                uuid: Uuid::from_u128(1),
                add: Some(("Steve".into(), vec![])),
                chat_session: Some(None),
                game_mode: Some(1),
                listed: Some(true),
                latency: Some(12),
                display_name: Some(Some(Component::text("S"))),
                list_priority: Some(0),
                show_hat: Some(true),
            }],
        });
        round_trip(&PlayerInfoUpdate {
            actions: player_info_actions::UPDATE_LATENCY,
            entries: vec![PlayerInfoEntry {
                uuid: Uuid::from_u128(2),
                latency: Some(5),
                ..Default::default()
            }],
        });
        round_trip(&PlayerPosition {
            teleport_id: 1,
            pos: DVec3::new(0.5, 70.0, 0.5),
            velocity: DVec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            flags: teleport_flags::VEL_X | teleport_flags::YAW,
        });
        round_trip(&RecipeBookAdd {
            body: Bytes::from_static(&[0, 1]),
        });
        round_trip(&RecipeBookRemove { body: Bytes::new() });
        round_trip(&RecipeBookSettings {
            body: Bytes::from_static(&[0; 8]),
        });
        round_trip(&RemoveEntities {
            entity_ids: vec![1, 2, 300],
        });
        round_trip(&Respawn {
            spawn: spawn_info(),
            data_kept: 3,
        });
        round_trip(&RotateHead {
            entity_id: 2,
            head_yaw: 33.75,
        });
        let p = SectionPos::new(-3, -4, 1_000_000);
        let u = SectionBlocksUpdate {
            section: SectionBlocksUpdate::pack_section(p),
            blocks: vec![(5 << 12) | (1 << 8) | (2 << 4) | 3],
        };
        assert_eq!(u.section_pos(), p);
        let c: Vec<_> = u.changes().collect();
        assert_eq!(c[0].pos, BlockPos::new(-48 + 1, -64 + 3, 16_000_000 + 2));
        assert_eq!(c[0].state, BlockStateId(5));
        round_trip(&u);
        round_trip(&ServerData {
            motd: Component::text("A Minecraft Server"),
            icon: None,
        });
        round_trip(&SetChunkCacheCenter { x: -1, z: 2 });
        round_trip(&SetChunkCacheRadius { radius: 10 });
        round_trip(&SetEntityData {
            entity_id: 1,
            metadata: vec![MetadataEntry {
                index: 0,
                value: MetadataValue::Byte(0),
            }],
        });
        round_trip(&SetEntityMotion {
            entity_id: 1,
            velocity: DVec3::ZERO,
        });
        round_trip(&SetEquipment {
            entity_id: 5,
            equipment: vec![(0, item()), (5, ItemStackData::default())],
        });
        round_trip(&SetExperience {
            bar: 0.5,
            level: 3,
            total: 40,
        });
        round_trip(&SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        });
        round_trip(&SetHeldSlot { slot: 4 });
        round_trip(&SetSimulationDistance { distance: 12 });
        round_trip(&SetTime {
            world_age: 1000,
            clocks: vec![ClockState {
                clock: 0,
                time: 6000,
                fraction: 0.0,
                rate: 1.0,
            }],
        });
        round_trip(&Sound {
            sound: IdOr::Id(10),
            category: 0,
            x: 8,
            y: 16,
            z: -8,
            volume: 1.0,
            pitch: 1.0,
            seed: 99,
        });
        round_trip(&SoundEntity {
            sound: IdOr::Inline(SoundEvent {
                name: Identifier::minecraft("x"),
                fixed_range: None,
            }),
            category: 5,
            entity_id: 3,
            volume: 0.5,
            pitch: 2.0,
            seed: 0,
        });
        round_trip(&StartConfiguration {});
        round_trip(&SystemChat {
            content: Component::text("sys"),
            overlay: true,
        });
        round_trip(&TabList {
            header: Component::text("h"),
            footer: Component::default(),
        });
        round_trip(&TeleportEntity {
            entity_id: 9,
            pos: DVec3::ONE,
            velocity: DVec3::ZERO,
            yaw: 1.0,
            pitch: 2.0,
            flags: 0,
            on_ground: true,
        });
        round_trip(&UpdateRecipes {
            body: Bytes::from_static(&[0, 0]),
        });
    }

    /// Regression: a normal-terrain chunk, built byte by byte in the 26.3
    /// layout (BitSets are trimmed little-endian byte arrays). Before the fix
    /// the masks were read as VarLongs and every such packet failed.
    #[test]
    fn level_chunk_with_light_golden_bytes() {
        let mut b = vec![];
        b.extend_from_slice(&(-3i32).to_be_bytes());
        b.extend_from_slice(&7i32.to_be_bytes());
        // Heightmaps: 1 entry, type 1 (world surface), 2 longs.
        b.extend_from_slice(&[1, 1, 2]);
        b.extend_from_slice(&0x0102_0304_0506_0708i64.to_be_bytes());
        b.extend_from_slice(&(-1i64).to_be_bytes());
        // Sections blob: 3 opaque bytes.
        b.extend_from_slice(&[3, 0xAA, 0xBB, 0xCC]);
        // One block entity at (x=5, z=9) in the chunk, y = -12, kind 33, empty compound.
        b.extend_from_slice(&[1, 0x59]);
        b.extend_from_slice(&(-12i16).to_be_bytes());
        b.extend_from_slice(&[33, 10, 0]);
        // Sky mask: sections 1..=2 and 25 (0x0200_0006): bytes 06 00 00 02.
        b.extend_from_slice(&[4, 0x06, 0x00, 0x00, 0x02]);
        // Block mask: bit 3 only: one byte.
        b.extend_from_slice(&[1, 0x08]);
        // Empty sky mask: bit 0: one byte. Empty block mask: none.
        b.extend_from_slice(&[1, 0x01, 0]);
        // Three sky arrays, one block array, each VarInt 2048 (0x80 0x10) + bytes.
        b.push(3);
        for v in [0xFFu8, 0x11, 0x22] {
            b.extend_from_slice(&[0x80, 0x10]);
            b.extend(std::iter::repeat_n(v, 2048));
        }
        b.push(1);
        b.extend_from_slice(&[0x80, 0x10]);
        b.extend(std::iter::repeat_n(0x33u8, 2048));

        let p = LevelChunkWithLight::decode_exact(&mut Reader::new(&b)).unwrap();
        let c = &p.chunk;
        assert_eq!(c.pos, ChunkPos::new(-3, 7));
        assert_eq!(c.heightmaps, vec![(1, vec![0x0102_0304_0506_0708, -1])]);
        assert_eq!(&c.sections[..], &[0xAA, 0xBB, 0xCC]);
        assert_eq!(
            c.block_entities[0].pos,
            BlockPos::new(-48 + 5, -12, 112 + 9)
        );
        assert_eq!(c.block_entities[0].kind, 33);
        assert_eq!(c.light.sky_mask, vec![0x0200_0006]);
        assert_eq!(c.light.block_mask, vec![8]);
        assert_eq!(c.light.empty_sky_mask, vec![1]);
        assert!(c.light.empty_block_mask.is_empty());
        assert_eq!(c.light.sky_arrays.len(), 3);
        assert_eq!(c.light.sky_arrays[2][0], 0x22);
        assert_eq!(c.light.block_arrays[0][2047], 0x33);
        // And our encoder writes the same bytes back.
        let mut w = Writer::new();
        p.encode(&mut w);
        assert_eq!(w.into_inner(), b);

        // light_update: VarInt chunk coords + the same light data layout.
        let mut l = vec![0x7F, 0x01]; // x = 127, z = 1
        l.extend_from_slice(&[2, 0xFE, 0x01, 0, 1, 0x01, 0, 1]);
        l.extend_from_slice(&[0x80, 0x10]);
        l.extend(std::iter::repeat_n(0xF0u8, 2048));
        l.push(0);
        let u = LightUpdate::decode_exact(&mut Reader::new(&l)).unwrap();
        assert_eq!(u.pos, ChunkPos::new(127, 1));
        assert_eq!(u.light.sky_mask, vec![0x01FE]);
        assert_eq!(u.light.empty_sky_mask, vec![1]);
        assert_eq!(u.light.sky_arrays.len(), 1);
    }

    #[test]
    fn add_entity_converts_to_spawn() {
        let a = AddEntity {
            entity_id: 7,
            uuid: Uuid::nil(),
            kind: 3,
            pos: DVec3::ONE,
            velocity: DVec3::ZERO,
            pitch: 1.0,
            yaw: 2.0,
            head_yaw: 3.0,
            data: 4,
        };
        let s = a.spawn();
        assert_eq!(s.id, EntityId(7));
        assert_eq!((s.yaw, s.pitch, s.head_yaw), (2.0, 1.0, 3.0));
    }
}
