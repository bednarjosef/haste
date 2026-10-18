//! The per-tick interaction step (player workstream): `Minecraft.tick`'s order — hotbar,
//! crosshair target, then `Minecraft.handleKeybinds` (the click loops, the held use,
//! `continueAttack`, `missTime`--), pick block, and drop / off-hand swap.

use haste_core::game::GameMode;
use haste_core::view::BlockView;
use haste_core::BlockPos;
use haste_protocol::packets::play::serverbound as sb;
use haste_protocol::ServerboundPlay;

use super::super::LocalPlayer;
use super::{action, InteractEnv, Target};

impl LocalPlayer {
    pub(crate) fn interact_step(
        &mut self,
        look: (f32, f32),
        sneak_key: bool,
        world: &dyn BlockView,
        env: &InteractEnv<'_>,
        out: &mut Vec<ServerboundPlay>,
    ) {
        let input = self.int.input;
        let last = std::mem::replace(&mut self.int.last, input);
        if self.int.tags.is_none() {
            self.int.tags = Some(haste_physics::mining::BlockTags::default());
        }

        // Minecraft.tick(): rightClickDelay-- happens before the key handling.
        if self.int.use_delay > 0 {
            self.int.use_delay -= 1;
        }

        // 1. Hotbar (handleKeybinds: the keyHotbarSlots loop).
        if let Some(slot) = input.hotbar {
            if slot <= 8 && slot as i32 != self.held_slot {
                self.held_slot = slot as i32;
                out.push(ServerboundPlay::SetCarriedItem(sb::SetCarriedItem {
                    slot: slot as i16,
                }));
                // A new tool restarts breaking (startDestroyBlock compares the held item).
                if let Some((_, face, _)) = self.int.destroying {
                    self.abort_destroy(face, out);
                }
            }
        }

        // 2. The crosshair target (Minecraft.pick). The held stack's components are decoded once
        // per stack change; nothing in the pick reads them, but the decode belongs with the
        // per-tick held-item upkeep. The pick runs in spectator too: `Minecraft.tick` calls it
        // whatever the mode, and `Minecraft.startAttack`'s spectator branch needs the target
        // (the entity to teleport to, or the miss).
        let held = self.main_hand().clone();
        self.int.refresh_components(held, env.data);
        self.int.target = self.pick(
            look,
            world,
            &env.data.shapes,
            env.data,
            env.entities,
            env.entity_types,
        );

        // 3. Minecraft.handleKeybinds: the click loops, the held use, then continueAttack.
        // `KeyMapping.consumeClick` returns once per queued press, so several clicks in one tick
        // each run. A driver that only reports the held button (no click queue) still gets one
        // click on the press edge, as consumeClick would on a fresh press.
        let mut attack_clicks = std::mem::take(&mut self.int.attack_clicks);
        let mut use_clicks = std::mem::take(&mut self.int.use_clicks);
        if attack_clicks == 0 && input.attack && !last.attack {
            attack_clicks = 1;
        }
        if use_clicks == 0 && input.use_item && !last.use_item {
            use_clicks = 1;
        }
        let mut instant_attack = false;
        // A use the server's synced flags started (`LivingEntity.onSyncedDataUpdated`): the
        // held item of that hand and its duration.
        if let Some(hand) = self.int.pending_use_hand.take() {
            let stack = self.hand(hand).clone();
            if let Some((duration, animation)) = self.int.use_data(&stack, env.data) {
                self.int.using = Some(super::UsingItem {
                    item: stack.item,
                    duration,
                    remaining: duration,
                    animation,
                });
            }
        }
        if self.int.is_using_item {
            // While an item is in use the clicks are consumed and do nothing; releasing the
            // button releases the item (MultiPlayerGameMode.releaseUsingItem), and a different
            // item in the hand stops the use without a packet
            // (`LivingEntity.updatingUsingItem`).
            if !input.use_item {
                self.release_using_item(out);
            } else if self
                .int
                .using
                .as_ref()
                .is_some_and(|u| self.main_hand().item != u.item)
            {
                self.int.using = None;
                self.int.is_using_item = false;
            }
        } else {
            // while (keyAttack.consumeClick()) instantAttack |= startAttack();
            for _ in 0..attack_clicks {
                instant_attack |= self.start_attack(world, env, out);
            }
            // while (keyUse.consumeClick()) startUseItem();
            for _ in 0..use_clicks {
                self.start_use_item(look, sneak_key, world, env, out);
            }
        }

        // if (keyUse.isDown() && rightClickDelay == 0 && !player.isUsingItem()) startUseItem();
        if input.use_item && self.int.use_delay == 0 && !self.int.is_using_item {
            self.start_use_item(look, sneak_key, world, env, out);
        }

        // `LivingEntity.tick` -> `updateUsingItem`: the remaining time ticks down each game tick,
        // including the tick the use started on.
        if let Some(mut u) = self.int.using {
            u.remaining -= 1;
            self.int.using = Some(u);
        }

        // continueAttack(screen == null && !instantAttack && keyAttack.isDown() && grabbed).
        if !instant_attack {
            self.continue_attack(input.attack, world, env, out);
        }

        // Minecraft.tick(): missTime-- after the key handling.
        if self.int.miss_time > 0 {
            self.int.miss_time -= 1;
        }

        // 4. Pick block (handleKeybinds: the keyPickItem loop). `Minecraft.pickBlockOrEntity`
        // passes `hasControlDown()` as `include_data` (Ctrl + middle click copies block-entity
        // data in creative).
        if input.pick_block && !last.pick_block {
            let include_data = input.pick_block_ctrl;
            match self.int.target {
                Some(Target::Block(h)) => {
                    out.push(ServerboundPlay::PickItemFromBlock(sb::PickItemFromBlock {
                        pos: h.pos,
                        include_data,
                    }))
                }
                Some(Target::Entity { id, .. }) => out.push(ServerboundPlay::PickItemFromEntity(
                    sb::PickItemFromEntity {
                        entity_id: id.0,
                        include_data,
                    },
                )),
                None => {}
            }
        }
        // 5. Drop (Q / Ctrl+Q) and off-hand swap (F): handleKeybinds' keyDrop / keySwapOffhand
        // loops run independently (all three in one tick is a drop, a stack drop and a swap);
        // gated on !isSpectator, not health. No target, pos and face zeroed as vanilla sends
        // it; the server answers with the slot updates.
        if self.game_mode != GameMode::Spectator {
            if input.drop_stack && !last.drop_stack {
                self.drop_item(true, out);
            }
            if input.drop_item && !last.drop_item {
                self.drop_item(false, out);
            }
            if input.swap_offhand && !last.swap_offhand {
                out.push(ServerboundPlay::PlayerAction(sb::PlayerAction {
                    status: action::SWAP_ITEM_WITH_OFFHAND,
                    pos: BlockPos::new(0, 0, 0),
                    face: 0,
                    sequence: 0,
                }));
            }
        }

        // The press-only fields are consumed; held buttons stay until the caller changes them.
        self.int.input.pick_block = false;
        self.int.input.pick_block_ctrl = false;
        self.int.input.drop_item = false;
        self.int.input.drop_stack = false;
        self.int.input.swap_offhand = false;
        self.int.input.hotbar = None;
    }
}
