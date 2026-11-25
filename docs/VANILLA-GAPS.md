# Vanilla gaps: what vanilla 26.3 does that Haste does not

Audit of 2026-10-19 on branch `vanilla-gap-audit`. Method: walk each vanilla client inventory
(packets, renderers, screens, HUD, keybinds, gameplay, world visuals, sounds) in the decompiled
26.3 client (`cache/source/26.3/client`), then look for the matching code in `crates/` by grep
and reading. "done" means wired into the running game (a Rust handler in the session's packet
groups, the tier-3 Java bridge, or the renderer), not just a function that exists. "missing"
means no non-test code found. This is a code audit: nothing here was verified by running the
game. Paths are relative to the repository root.

Packet dispatch (where a packet goes): `crates/haste-client/src/net.rs:392-512` routes to the
group modules (one per vanilla concept, mirroring `ClientPacketListener`), and
`crates/haste-client/src/ui/packets.rs:9-68` forwards tier-3 packets (screens, chat, scoreboards,
recipes, advancements, ...) whole to the real vanilla Java in the JVM.

## 1. Clientbound play packets (`ClientPacketListener.handle*`)

All 127 play packets decode (`crates/haste-protocol/src/packets/play/clientbound/mod.rs:69-92`);
handling is what varies. Debug-only vanilla features (F3+b chunk/entity values, game test
highlight, debug sample) are marked "debug" and are fine to ignore.

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| `handleLogin` | done | crates/haste-client/src/net.rs:402 | — |
| `handleAddEntity` | done | matched in crates/haste-client/src/net/entities.rs | — |
| `handleSetEntityMotion` | done | crates/haste-client/src/net/entities.rs | — |
| `handleSetEntityData` | done (Rust + forwarded) | crates/haste-client/src/net.rs:487; ui/packets.rs:63 | — |
| `handleEntityPositionSync` | done | crates/haste-client/src/net/entities.rs | — |
| `handleTeleportEntity` | done | crates/haste-client/src/net/entities.rs | — |
| `handleTickingState` (frozen sections) | missing | none found | low (entities above simulation distance keep moving locally) |
| `handleTickingStep` | missing | none found | low |
| `handleSetHeldSlot` | done | crates/haste-client/src/player.rs (SetHeldSlot arm); ui/packets.rs:302 | — |
| `handleMoveEntity` (pos/rot/posrot) | done | crates/haste-client/src/net/entities.rs | — |
| `handleMinecartAlongTrack` (smooth minecart steps) | done | crates/haste-client/src/net/minecart.rs; the walk in crates/haste-entity/src/new_minecart_behavior.rs | — |
| `handleRotateMob` | done | crates/haste-client/src/net/entities.rs (RotateHead) | — |
| `handleRemoveEntities` | done | crates/haste-client/src/net/entities.rs | — |
| `handleMovePlayer` (PlayerPosition) | done | crates/haste-client/src/player.rs (PlayerPosition arm) | — |
| `handleRotatePlayer` | done | crates/haste-client/src/player.rs (PlayerRotation arm) | — |
| `handleChunkBlocksUpdate` | done | crates/haste-client/src/net/world.rs (SectionBlocksUpdate) | — |
| `handleLevelChunkWithLight` | done | crates/haste-client/src/net/world.rs | — |
| `handleChunksBiomes` | done | crates/haste-client/src/net/world.rs | — |
| `handleForgetLevelChunk` | done (Rust + Java store) | crates/haste-client/src/net/world.rs; ui/packets.rs:51 | — |
| `handleBlockUpdate` | done | crates/haste-client/src/net/world.rs | — |
| `handleConfigurationStart` | done | crates/haste-client/src/net/misc.rs | — |
| `handleTakeItemEntity` | done | crates/haste-client/src/net/pickup.rs | — |
| `handleSystemChat` | done (Rust + Java) | crates/haste-client/src/net/chat.rs; Handlers.java:44 | — |
| `handlePlayerChat` | done (Rust + Java) | crates/haste-client/src/net/chat.rs; Handlers.java:53 | — |
| `handleDisguisedChat` | done | crates/haste-client/src/net/chat.rs; Handlers.java:58 | — |
| `handleDeleteChat` | done (Java) | ui/packets.rs:13; Handlers.java:63 | — |
| `handleAnimate` (crit particles on others, wake up) | missing | none found | low |
| `handleHurtAnimation` | done | crates/haste-client/src/net/entities.rs | — |
| `handleSwingAnimation` | done | crates/haste-entity/src/store.rs:115-117 | — |
| `handleSetTime` | done | crates/haste-client/src/net/misc.rs; ui/packets.rs:315 | — |
| `handleSetSpawn` (SetDefaultSpawnPosition) | missing | none found | low (compass spins to wrong target) |
| `handleSetEntityPassengersPacket` | done | crates/haste-client/src/net/entities.rs (SetPassengers); "mount.onboard" hint Bridge.java:264 | — |
| `handleAddTransientBlock` | missing | none found | low (26.3 feature; server-set transient block) |
| `handleEntityLinkPacket` (leashes) | missing | none found | medium (leash lines never attach, see §9) |
| `handleEntityEvent` | done | crates/haste-client/src/net/entities.rs; ui/packets.rs:309 | — |
| `handleDamageEvent` | done | crates/haste-client/src/net/damage.rs:300 | — |
| `handleSetHealth` | done | crates/haste-client/src/player.rs (SetHealth); net.rs:463 | — |
| `handleSetExperience` | done | crates/haste-client/src/ui/packets.rs:256 | — |
| `handleRespawn` | done | crates/haste-client/src/net.rs:290-301 | — |
| `handleExplode` (camera shake, particles, knockback) | missing | none found | high (TNT/creepers: no shake, no emitter particle) |
| `handleMountScreenOpen` | missing | none found | low (riding inventory open on mount) |
| `handleOpenScreen` | done (Java `MenuScreens.create`) | ui/packets.rs:35; Handlers.java:284-286 | — |
| `handleContainerSetSlot` | done (player + Java) | crates/haste-client/src/player.rs; Handlers.java:363 | — |
| `handleSetCursorItem` | done (Java) | Handlers.java:368 | — |
| `handleSetPlayerInventory` | done (player + Java) | crates/haste-client/src/player.rs; Handlers.java:373 | — |
| `handleContainerContent` | done (player + Java) | crates/haste-client/src/player.rs; Handlers.java:358 | — |
| `handleOpenSignEditor` | done | ui/packets.rs:48; Handlers.java:291 | — |
| `handleBlockEntityData` | done (Rust + Java) | crates/haste-client/src/net/block_entity_types.rs; Handlers.java:308 | — |
| `handleContainerSetData` | done (Java) | Handlers.java:377 | — |
| `handleSetEquipment` | done | crates/haste-client/src/net/entities.rs | — |
| `handleContainerClose` | done (Java) | Handlers.java:383 | — |
| `handleBlockEvent` | done | crates/haste-client/src/net/block_events.rs | — |
| `handleBlockDestruction` | done | crates/haste-client/src/net/world.rs | — |
| `handleGameEvent` | done | crates/haste-client/src/net/player_state.rs | — |
| `handleMapItemData` (maps) | missing | none found (decodes only: haste-protocol .../map.rs:53) | high (held maps, item frame maps, decorations) |
| `handleLevelEvent` | done | crates/haste-client/src/net/effects.rs | — |
| `handleUpdateAdvancementsPacket` | done (Java) | ui/packets.rs:31; Handlers.java:247 | — |
| `handleSelectAdvancementsTab` | done (Java) | Handlers.java:252 | — |
| `handleCommands` | done (Java) | Handlers.java:274 | — |
| `handleStopSoundEvent` | done | crates/haste-client/src/net (StopSound arm) | — |
| `handleCommandSuggestions` | done (Java) | Handlers.java:279 | — |
| `handleUpdateRecipes` | done (Java) | ui/packets.rs:36; Handlers.java (recipe book) | — |
| `handleLookAt` (PlayerLookAt, /tp facing) | missing | none found | low |
| `handleTagQueryPacket` | missing (debug) | none found | none |
| `handleAwardStats` | done (Java) | Handlers.java:257 | — |
| `handleRecipeBookAdd/Remove/Settings` | done (Java) | ui/packets.rs:54-58 | — |
| `handleUpdateMobEffect` | done (Rust + Java) | crates/haste-client/src/net/effects.rs; Handlers.java:215 | — |
| `handleUpdateTags` | done | crates/haste-client/src/net/registries.rs; ui/packets.rs:44 | — |
| `handlePlayerCombatEnd` / `handlePlayerCombatEnter` | missing | none found | none (no-op in vanilla) |
| `handlePlayerCombatKill` | done | crates/haste-client/src/ui/packets.rs:279; Handlers.java:399 | — |
| `handleChangeDifficulty` | missing | none found | low (difficulty icon on death screen) |
| `handleSetCamera` (spectate another entity) | done (no creeper/spider/enderman post effect) | crates/haste-client/src/net/entities.rs (`handle_set_camera`), crates/haste-client/src/game/camera_entity.rs | low (the spectated mob's post effect is not drawn) |
| `handleInitializeBorder` / `SetBorderCenter` / `SetBorderSize` / `SetBorderLerpSize` | done (collision only) | crates/haste-client/src/net/effects.rs:45-51 | partial: the wall itself is not drawn (see §9) |
| `handleSetBorderWarningDistance` / `WarningDelay` | missing | none found; crates/haste-client/src/world_border.rs:7 keeps only centre+diameter | low (no red vignette near the border) |
| `handleCustomPayload` (play phase) | missing | none found (config-phase payload handled) | low |
| `handleCooldown` (item cooldowns) | missing | none found | medium (pearl/chorus cooldown not shown, no HUD sweep) |
| `handleCustomChatCompletions` | done (Java) | ui/packets.rs:14 | — |
| `handleDebugSample` / `DebugChunkValue` / `DebugBlockValue` / `DebugEntityValue` / `DebugEvent` / `handleGameTestHighlightPos` / `handleTestInstanceBlockStatus` | missing (debug) | none found | none |
| `handleDeleteChat` see above | done | | |
| `handleDisconnect` | done | crates/haste-client/src/net/misc.rs | — |
| `handleLevelParticles` | done | crates/haste-client/src/net/particles.rs:20 | — |
| `handleLightUpdate` | done | crates/haste-client/src/net/world.rs | — |
| `handleMerchantOffers` | done (Java) | Handlers.java:387 | — |
| `handleMoveVehicle` | done | crates/haste-client/src/net/entities.rs | — |
| `handleOpenBook` | done (Java) | Handlers.java:318 | — |
| `handlePing` / `handlePongResponse` | done (connection layer) | crates/haste-protocol connection | — |
| `handlePlaceGhostRecipe` | done (Java) | ui/packets.rs:58 | — |
| `handlePlayerAbilities` | done | crates/haste-client/src/player.rs (PlayerAbilities arm) | — |
| `handlePlayerInfoUpdate` / `Remove` | done (Rust + Java) | crates/haste-client/src/net/chat.rs; Handlers.java:103,124 | — |
| `handleRemoveMobEffect` | done (Rust + Java) | crates/haste-client/src/net/effects.rs; Handlers.java:223 | — |
| `handleResetScore` / `handleSetScore` / `handleAddObjective` / `handleSetDisplayObjective` / `handleSetPlayerTeamPacket` | done (Java) | Handlers.java:140-186 | — |
| `handleResourcePackPush` / `Pop` | done | crates/haste-client/src/net/pack_prompt.rs | — |
| `handleProjectilePowerPacket` | missing | none found | low (wind-charge FOV kick) |
| `handleServerData` | missing | none found | low (connecting-screen MOTD) |
| `handleSetChunkCacheRadius` / `SetChunkCacheCenter` | done | crates/haste-client/src/net/world.rs | — |
| `handleSetSimulationDistance` | missing | none found | low (server render distance ignored) |
| `handleSoundEvent` / `SoundEntity` | done | crates/haste-client/src/net (Sound, SoundEntity arms) | — |
| `handleStartConfiguration` | done | crates/haste-client/src/net/misc.rs | — |
| `handleStoreCookie` / `handleTransfer` | done | crates/haste-client/src/net/misc.rs (Transfer arm) | — |
| `handleTabList` (header/footer) | done (Java) | ui/packets.rs:21; Handlers.java:86 | — |
| `handleSetTitlesAnimation` / `SetTitleText` / `SetSubtitleText` / `SetActionBarText` / `ClearTitles` | done (Java) | ui/packets.rs:16-19; Handlers.java:70-78 | — |
| `handleWaypoint` | missing | none found | low (26.3 tracked-waypoint HUD) |
| `handleShowDialog` / `handleClearDialog` | missing | none found | medium (dialog screens, the quick-actions key does nothing) |
| `handleCustomReportDetails` / `handleLowDiskSpaceWarning` / `handleServerLinks` | partial | ServerLinks/config-phase done (ui/packets.rs `joined`); report details, disk warning missing | none |
| `handleBundlePacket` (delimiter) | done | crates/haste-client/src/net/driver.rs | — |

Count: about 85 done, 4 partial (border visuals, report details), 16 missing with player impact,
plus debug-only misses. The misses that matter: `Explode`, `MapItemData`, `Cooldown`,
`SetEntityLink`, `MoveMinecartAlongTrack`, `ShowDialog`, `TickingState`.

## 2. Entity renderers (`EntityRenderers` registrations, 161 types) and layers

What exists: all ~91 living mob types render with vanilla models from the recorder dump
(hand table `crates/haste-entity/src/registry.rs:141-...`, one entry per vanilla renderer with
its `ModelLayers` and texture; e.g. creeper registry.rs:197, ender_dragon registry.rs:248),
with babies (registry.rs:982), variants (tropical fish, shulker, dyes: variants/), overlay
layers such as sheep wool and horse markings (store.rs:1100-1102, overlays drawn at
crates/haste-render/src/entity.rs:501-518), equipment on humanoids — held items, armour,
elytra wings, custom head (crates/haste-render/src/entity/equipment.rs:11-156, called at
entity.rs:520) — item entities, item frames, paintings, lightning, experience orbs
(registry.rs:935-947), fire overlay (entity.rs:521-524), name tags with the 64/32-block rule
(entity.rs:528-541), shadows (entity.rs; radius table registry.rs:1012-1143), and pose
animation (crates/haste-entity/src/animation.rs).

Other players' held items, which the lead found missing, are wired on this branch:
`emit_equipment` draws both hands (equipment.rs:19-42), fed from `SetEquipment`
(crates/haste-entity/src/local_player.rs:181-213) — code-verified only, not visually checked.

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| `AbstractBoatRenderer` (all boats, rafts, chest boats) | missing (generic box) | no boat model: no `#main` entry in registry.rs TABLE; only shadow radius registry.rs:1114-1133 | high |
| `AbstractMinecartRenderer` (all minecarts) | missing (generic box) | shadow radius only, registry.rs:1134-1139 | high |
| `ArmorStandRenderer` | missing (generic box) | not in TABLE; only "living" flag store.rs:298 | high (armour stands everywhere in survival) |
| `TntRenderer` (flashing white) | missing (generic box) | shadow radius only, registry.rs:1140 | medium |
| `FallingBlockRenderer` (renders the falling block's model) | missing (generic box) | none found in crates/haste-render/src/entity.rs or store.rs | high (sand, gravel, anvils, concrete powder) |
| `TippableArrowRenderer`, `SpectralArrowRenderer` (arrow model, glowing) | missing (generic box) | no TABLE entry | high |
| `ThrownItemRenderer` (snowball, egg, ender pearl, potions, eyes, fireballs) | missing (generic box) | no TABLE entry | high |
| `TridentRenderer` | missing (generic box) | none found | medium |
| `FishingHookRenderer` (bobber + line to the rod) | missing (generic box) | pick.rs:218 names the type only; no rendering | medium |
| `FireworkEntityRenderer` (rocket + flight trail) | missing (generic box) | no TABLE entry | medium |
| `EndCrystalRenderer` (crystal + beam to the dragon) | missing (generic box) | shadow radius only, registry.rs:1140 | medium (End fight) |
| `WindChargeRenderer`, `BreezeWindChargeRenderer` | missing (generic box) | none found | medium |
| `AreaEffectCloudRenderer` | missing (generic box; vanilla is translucent) | none found | low |
| `EvokerFangsRenderer` (bite animation, `EvokerFangsModel`, status 4 bite sound) | partial: vanilla model and bite through the generic model path (registry.rs `minecraft:evoker_fangs`, crates/haste-entity/src/evoker_fangs.rs, evoker_fangs_model.rs, render.rs:134-207); invisible before the bite; not batched in a dedicated renderer; bite sound not silent-aware; on-screen check inconclusive (bite seen in the log, fangs not clearly seen in screenshots) | low |
| `LlamaSpitRenderer` (`llama_spit#main` model, fixed transform, no shadow) | partial: vanilla model and transform through the generic model path (llama_spit_model.rs, render.rs:134-207); on-screen check inconclusive (a small dark shape, not confirmed as the spit) | low-medium |
| `WitherSkullRenderer`, `DragonFireballRenderer`, `ShulkerBulletRenderer`, `OminousItemSpawnerRenderer`, `EyeOfEnderRenderer`, `ExperienceBottleRenderer` | missing (generic box) | none found | low-medium |
| Display entities: `DisplayRenderer.Block/Item/Text` | missing (generic box for block/item, nothing for text) | tests say so: crates/haste-entity/src/tests/mod.rs:212 | medium (datapacks, 26.3 features, mods later) |
| `LeashKnotRenderer` + leash lines between entities | missing | no non-test match for "leash" in haste-render | medium |
| `GuardianRenderer`/`ElderGuardianRenderer` attack beam | missing | guardian model exists (registry.rs:317), no beam code | medium |
| `CushionRenderer`, `MannequinRenderer`, `ZombieNautilusRenderer`, `ParchedRenderer`, `CopperGolemStatueRenderer` (26.3 types) | partial | some are in TABLE (zombie_nautilus registry.rs:788, parched registry.rs:457); statue, cushion not checked individually | low |
| `ItemInHandLayer`, `HumanoidArmorLayer`, `WingsLayer` (elytra), `CustomHeadLayer`, `Deadmau5FeatureRenderer` | done | equipment.rs:11-156; store.rs:1065-1069 | — |
| `SheepWoolLayer`, horse markings, cat/dye overlays | done | store.rs:1100-1102; offscreen test crates/haste-render/tests/entity_offscreen.rs:1145 | — |
| `ArrowLayer` (arrows stuck in a body) | missing | no match for arrows-stuck handling | low |
| `BeeStingerLayer` | missing | none found | low |
| `ParrotOnShoulderLayer` | missing | none found (riding.rs:354 names parrot as passenger only) | low |
| `WolfCollarLayer`, wolf/dog armour | missing | no match ("collar" only in store.rs comment) | low |
| Horse armour and saddles (`HorseArmorLayer`, body slot) | missing | equipment.rs:44-48 iterates Chest/Legs/Feet/Head only; saddle slot exists for logic only (crates/haste-data/src/components/equippable.rs:30) | medium (armoured horses look bare) |
| `CapeLayer` (other players' capes + elytra texture) | done | crates/haste-render/src/entity/cape.rs; avatar.rs:135 | — |
| `SpinAttackEffect` (trident riptide spin trail) | missing | none found | low |
| `PlayerItemLayer`-style held-block rendering (players holding blocks) | done (item model path) | equipment.rs via item model | — |

Count: done 14 rows, missing/partial ~24 rows. The pattern: every entity with a bespoke
non-mob renderer (boats, minecarts, projectiles, stands, falling blocks, displays) renders as
a textured box today.

## 3. Block entity renderers (`BlockEntityRenderers`, 28 registrations)

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Beacon (beam), bell, chest/trapped/ender chest + double chests, conduit, decorated pot, enchanting table, end gateway, end portal, lectern, piston head, shulker box, sign + hanging sign, skull, banner, campfire items, spawner + trial spawner (spinning mob), shelf, brushable block item | done | crates/haste-render/src/blockentity.rs:283-330 (kind table), one module per renderer in crates/haste-render/src/blockentity/ | — |
| Bed (rendered as a block model, no bespoke renderer) | done (block model) | mesher; no renderer in vanilla either | — |
| `CopperGolemStatueRenderer` | missing | no match | low (26.3) |
| `StructureBlockRenderer`, `TestInstanceRenderer` | missing | no match | none (creative/debug) |

Count: 25 done, 3 missing (2 negligible).

## 4. Particles (`ParticleResources.registerProviders`, 128 providers)

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Particle definitions/textures for all types (sprite atlas) | done | crates/haste-render/src/particles/resources.rs:34-86 | — |
| Terrain particles (breaking, mining, landing) and item particles | done | crates/haste-render/src/block_particles.rs; particles/item_particle.rs, item_sprite.rs | — |
| Per-type behaviour (gravity, friction, lifetime, colour, growth) | partial: 19 ids ported | crates/haste-render/src/particles/providers.rs:94-... ("flame", "crit", "portal", ...); everything else uses the generic `Particle` DEFAULT | medium (dripstone drips, electric spark, sculk soul, firework sparkles, shriek, gust, snowflake, wax, spores behave generically) |
| `ItemCobwebParticle`-style emissive/full-light handling | done for the ported ids | providers.rs (full_light) | — |

Count: textures all done; behaviour 19/128 ported, the rest render as generic quads with the
right texture.

## 5. Screens (`net/minecraft/client/gui/screens/**`) — tier-3 real Java

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| All container screens (`MenuScreens.create`): chests, crafting, furnace, anvil, enchanting, brewing, hopper, villager trades, horse, loom, stonecutter, smithing, beacon, shulker | done | open_screen forwarded (ui/packets.rs:35); Handlers.java:284-286 | — |
| Inventory + crafting grid, creative inventory with tabs/search | done | ui/input.rs:106 (E); Handlers.java MenuScreens; README claims script-checked | — |
| Chat, command with suggestions, social/report chat screens | done | ui/input.rs:104-105; chat_session module; Handlers.java:274-279 | — |
| Sign edit, book view/edit, command block edit | done | Handlers.java:290-347 | — |
| Death screen and respawn | done | Handlers.java:401 | — |
| Pause menu, options and every sub-screen, language, controls, resource packs, server list, select/create world | done | ui/input.rs:107; ScreenKind enum crates/haste-jvm/src/gui.rs:39-76; README script-checked | — |
| Statistics, advancements, game rules screens | done | Handlers.java:257-268 | — |
| Recipe book overlay | done | ui/packets.rs:52-58; crates/haste-jvm/java/haste/gui/RecipeBookImpl.java | — |
| Spectator menu (teleport hotbar) | missing | no spectator GUI beyond flight: crates/haste-jvm/java/haste/gui/PlayerImpl.java:20 | medium (spectators cannot teleport) |
| Dialog screens (26.3 `show_dialog`, quick actions) | missing | ShowDialog not handled (see §1) | medium |
| Horse inventory when mounting | partial | MountScreenOpen packet unhandled; container screens open when the server sends open_screen | low |

Count: ~15 done, 2 missing (spectator menu, dialogs), 1 partial.

## 6. HUD / `Gui` elements — the real vanilla `Gui` runs in the JVM

`Proxies.hud` is vanilla's `Gui`, ticked and rendered every frame (`Bridge.java:190`,
`Bridge.java:161`), so its elements come with it. Checked individually:

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Hotbar, health, hunger, armour, air, experience, crosshair + attack indicator | done | Java Gui (Bridge.java:190); vitals mirrored ui/packets.rs:251-269; crosshair pick feeds it (crates/haste-client/src/player/interact/pick.rs:55) | — |
| Boss bars, titles/subtitles/action bar, scoreboard sidebar, tab list | done | forwarded ui/packets.rs:16-28; Handlers.java:70-186 | — |
| Effects icons (inventory/HUD) | done | mob-effect forwarding Handlers.java:215-227 | — |
| Mount bar (horse jump), spectator bar | done (mount); spectator menu missing | crates/haste-jvm/proxies/mount_hud.rs; see §5 | — |
| Toasts (recipes, advancements, system) | done | ClientImpl.java:383 (ToastManager) | — |
| Vignette/pumpkin/powder snow/portal/fire/in-wall/spyglass/nausea overlays | done | crates/haste-client/src/screen_effects/ (camera_overlay, fire, in_block, portal, powder_snow, spyglass, water, confusion) | — |
| Subtitles (sound captions) | done | real vanilla `SubtitleOverlay`; Rust's played sounds reach it once per frame: haste-audio/src/sound_event_listener.rs, haste-jvm/src/sound_manager.rs, SoundImpl.java | — |
| Debug screen (F3) | done | ui/input.rs:113-116; DebugImpl.java | — |
| World border red vignette + wall | missing | no border draw code in haste-render; world_border.rs:7 keeps collision only | medium |
| Item cooldown sweep on the hotbar icon | missing | Cooldown packet unhandled (see §1) | medium |

## 7. Keybinds (`Minecraft.handleKeybinds`, `KeyMapping`s)

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Movement/action mappings mirrored into real vanilla `KeyMapping`s (rebindable, options screens) | done | ui/input.rs:46-70; Java mirrors: crates/haste-jvm/src/gui/input.rs | — |
| Chat (T, /), inventory (E), pause (Esc), F1 hide HUD, F3 debug, Tab list, hotbar 1-9 + wheel | done | crates/haste-client/src/ui/input.rs:89-122 | — |
| Attack/use/pick (mouse), use-hold delay, item-use release | done | crates/haste-client/src/player/interact/step.rs | — |
| Drop (Q, Ctrl+Q), swap offhand (F) | done | ui/input.rs:41-43 | — |
| F5 perspective cycle, F11 fullscreen, F2 screenshot | done | crates/haste-client/src/app.rs:3603-3621 | — |
| Spectator scroll fly-speed | done | crates/haste-client/src/player/spectator.rs:34 | — |
| Spectator hotbar menu key | missing | no handling (see §5) | medium |
| Quick actions key | missing | ShowDialog unhandled | low |
| Cinematic camera (smooth camera key) | missing | no match beyond options field | low |

## 8. Gameplay (`LocalPlayer` / `MultiPlayerGameMode`)

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Attack, use, interact per item class; click delays, cooldowns, swing timing | done | crates/haste-client/src/player/interact/ (attack, use_item, place, pick, drop, destroy, step) | — |
| Bow/crossbow/trident/spyglass/shield use durations and animations; consumables with `use_effects` slowdown | done | crates/haste-client/src/player/interact/use_item.rs:90-100, 147-173 | — |
| Spawn eggs, ender pearls, buckets, boats as items (client side is `use_item_on` + server answer) | done | use_item.rs:480-500 (entity spawn, server-authoritative) | — |
| Item cooldown gating (pearls, chorus) | missing | Cooldown packet unhandled; no gating beyond server refusal | medium |
| Elytra flight + firework boost | done | crates/haste-physics/src/elytra.rs; session fireworks net.rs:414 | — |
| Riding: horses (jump bar, hearts), pigs, striders, boats, camels; seat positions | done | crates/haste-client/src/{horse,pig,strider,boat}.rs; crates/haste-entity/src/riding.rs | — |
| Movement effects: levitation, slow falling, jump boost, speed (via synced attributes) | done | crates/haste-physics/src/movement.rs:576-583, 526; net/player_state.rs attributes | — |
| Blindness/darkness fog; water/lava/powder snow/nether/end fog | done | crates/haste-render/src/fog/{blindness,water,lava,powder_snow,atmospheric}.rs | — |
| Dolphin's grace | partial | no explicit code; swim speed modifiers would have to arrive via `UpdateAttributes` — not verified | low |
| Spectator camera (`SetCamera` spectate) | done | crates/haste-client/src/game/camera_entity.rs, crates/haste-client/src/player/camera.rs | — |
| Totem of undying animation (particles, hand flash) | missing | only the particle wire id exists | medium |
| Trident riptide spin | missing | none found | low |

## 9. World visuals

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Weather (rain/snow/thunder), weather fog, lightning bolts | done | crates/haste-client/src/net/weather.rs; crates/haste-render/src/weather.rs; LightningBolt decoration registry.rs:944 | — |
| Sky: sun, moon, stars, clouds, end sky, nether fog colours | done | crates/haste-render/src/{sky,stars,clouds}.rs; sky.rs:96, 310-355 | — |
| Water/lava rendering, flow, corner heights | done | crates/haste-render/src/fluid.rs | — |
| Block break cracks, block outline | done | BlockDestruction in net/world.rs; outline boxes renderer/frame_setup.rs:61 | — |
| Entity shadows, entity fire overlay, hurt flash | done | entity/shadow.rs; entity.rs:521-524 | — |
| Item frames (incl. glow), paintings, dropped items with bob/spin/copies | done | registry.rs:941-945; crates/haste-render/src/entity/item.rs | — |
| Maps in hand / in item frames, map decorations | missing | MapItemData unhandled; no map renderer | high |
| Leash rendering (leash knot + line) | missing | SetEntityLink unhandled; no renderer | medium |
| Fishing line | missing | no renderer (see §2) | medium |
| Explosion camera shake + emitter particle | missing | Explode unhandled | high |
| World border wall + red warning vignette | missing | no renderer; collision only | medium |
| First-person hand: swing, equip, bob, eat/drink, bow draw FOV | done | crates/haste-render/src/hand/; crates/haste-client/src/net/fov.rs:24 | — |
| View bobbing, hurt tilt, death camera | done | renderer/camera.rs; net.rs:463-467 (hurtTo) | — |

## 10. Sounds

| Vanilla thing | Haste status | Evidence | Player impact |
|---|---|---|---|
| Server-sent sounds (positional, entity, stop), volume categories, spatialization | done | crates/haste-client/src/net (Sound/SoundEntity/StopSound arms); crates/haste-audio/src/{spatial,channels}.rs | — |
| Own footsteps, mining, breaking, placing, falling | done | crates/haste-client/src/sound/{steps,own_block,falls}.rs | — |
| Entity hurt/death sounds following entities | done | crates/haste-client/src/net/damage.rs:117 | — |
| Background music (`MusicManager`, biome/situation music) | done | crates/haste-audio/src/music.rs | — |
| Ambient loops: cave ambience, underwater loops, biome ambience (Nether etc.) | missing | only a name-resolution test mentions ambient.cave (crates/haste-client/src/sound/tests.rs:76) | medium-high (the world feels silent) |
| Minecart/elytra/boat loop sounds | missing | none found | low |

## README claims found wrong or overstated

- "Armour, held items, heads and elytra on mobs and players" (README:144): held items on other
  players ARE wired on this branch (equipment.rs:19-42, fed from SetEquipment) — the lead's
  finding may predate the current branch or need a visual re-check. However "on mobs" is
  overstated: horse armour, wolf armour and saddles are not drawn (equipment.rs:44-48 covers
  Chest/Legs/Feet/Head on humanoids only).
- "Particles: ... with vanilla's motion, size and lifetime" (README:146): true for the 19 ported
  ids; the other ~110 types use the generic default behaviour (providers.rs:94).
- "Terrain: ... every packet decodes" (README:111) is about decoding; several packets are
  decoded but ignored (see §1), which the 90 % figure does not convey.
- The entity claim "vanilla's models for all 91 mob types" (README:145) is accurate for
  living mobs; entities without mob renderers (boats, minecarts, arrows, armour stands,
  falling blocks, TNT) are boxes and are not covered by any README item.

## Top 30 to do next (ranked, high impact first)

Each item names the vanilla class(es) to port. "One worker" = a single focused task of the
usual size; otherwise it says how to split.

1. **Maps**: `handleMapItemData` + `MapRenderer`/`MapItemSpecialRenderer` (held), map in item
   frames, decorations. One worker (packet store + a new renderer module + item-frame hookup).
2. **Explosion feedback**: `handleExplode` — camera shake (`GameRenderer.bobHurt`-style offset
   and the 26.3 shake), the explosion emitter particle, player knockback nudge. One worker.
3. **Boat and raft models**: `AbstractBoatRenderer` (+ chest boat variants). One worker; the
   chest-boat chest can follow.
4. **Minecart models**: `AbstractMinecartRenderer` (+ the five variants' cargo). One worker.
5. **Falling blocks as their blocks**: `FallingBlockRenderer` reads the entity's block state
   and renders the block model at the entity's position. One worker.
6. **TNT with the flash**: `TntRenderer` (white overlay at fuse ticks 5 and below). One worker.
7. **Armour stands**: `ArmorStandRenderer` (humanoid model with vanilla's pose, no base plate
   in the model, no shadow) + armour on stands. One worker.
8. **Arrows and tipped arrows**: `TippableArrowRenderer` (+ spectral arrow glow). One worker.
9. **Thrown items**: `ThrownItemRenderer` for snowball, egg, ender pearl, eyes of ender,
   potions, fireballs, experience bottles. One worker, shares the item-model path.
10. **Fishing hook and line**: `FishingHookRenderer` (bobber model + the line to the player's
    rod). One worker, depends on #9's item path only lightly.
11. **Firework rockets**: `FireworkEntityRenderer` (rocket + trail particles) — pairs with
    the existing client-side firework boost. One worker.
12. **Item cooldowns**: `handleCooldown` + hotbar sweep overlay (`Gui.renderCooldowns`) + use
    gating. One worker.
13. **Leashes**: `handleEntityLinkPacket` + `LeashKnotRenderer` + the leash line between entity
    and holder. One worker.
14. **Ambient sound loops**: cave/underwater/biome ambience (`AmbientLoopHandler`,
    `BiomeAmbientSoundsHandler`, `UnderwaterAmbientSoundHandler` equivalents). One worker
    (needs a small ambient-sound scheduler in haste-client/src/sound/).
15. ~~**Minecart movement smoothing**: `handleMinecartAlongTrack`.~~ Done (`NewMinecartBehavior`'s
    track walk, the riding camera and the passenger offset).
16. **World border visuals**: `WorldBorderRenderer` (the wall) + warning-distance vignette
    (needs `SetBorderWarningDistance/Delay` stored). One worker.
17. **End crystal**: `EndCrystalRenderer` (crystal model + beam to the dragon; the beam pairs
    with the dragon's `EnderDragonRenderer` beam, which is also missing). One worker.
18. **Spectator completeness**: `handleSetCamera` (spectate entity camera) + spectator menu
    (`SpectatorGui` hotbar teleport). One worker; can split camera vs menu.
19. **Guardian attack beam**: the `GuardianRenderer` laser beam between guardian and target.
    One worker.
20. **Wind charge / breeze wind charge**: `WindChargeRenderer` + `ProjectilePower` FOV kick.
    One worker.
21. **Particle behaviour sweep**: port the remaining high-visibility providers (drip
    `DripParticle`, `ElectricSparkParticle`, firework `SimpleAnimatedParticle`, sculk
    `SculkParticle`, `ShriekParticle`, gust, snowflake, wax). Split: 3-4 workers of ~10 types
    each, one module per family next to particles/providers.rs.
22. **Horse armour, saddles, wolf armour layers**: `HorseArmorLayer` body slot + `WolfArmorLayer`
    + saddle models. One worker (equipment plumbing for the body slot exists in the protocol).
23. **Display entities**: block/item/text display rendering (`DisplayRenderer` subtree). Split:
    one worker per display kind (transform/interpolation plumbing is the shared part).
24. **Totem animation**: totem pop particles, hand item animation, screen flash
    (`GameRenderer.displayItemActivation`). One worker.
25. **Dialogs**: `handleShowDialog`/`ClearDialog` + opening vanilla `DialogScreen`s through the
    JVM bridge + quick-actions key. One worker.
26. **Trident**: `TridentRenderer` + riptide spin trail (`SpinAttackEffect`). One worker.
27. **Arrows stuck in bodies + bee stinger + parrot on shoulder**: the small `RenderLayer`s
    (`ArrowLayer`, `StingerLayer`, `ParrotOnShoulderLayer`). One worker for all three.
28. **Spawn/compass/settle packets**: `handleSetSpawn` (compass target), `handleSetSimulationDistance`,
    `handleLookAt`, `handleChangeDifficulty`, `handleMountScreenOpen`, `handleTickingState/Step`,
    `handleProjectilePower`, `handleWaypoint`, `handleServerData`, `handleAddTransientBlock`.
    Split: 2 workers (a "small packets" batch each); all are one match arm plus a small effect.
29. **Animate packet**: crit particles on remote attacks, wake-up animation. One worker, small.
30. ~~**Subtitles**~~: done. Rust's sound plays reach the real Java `SubtitleOverlay`
    (`SoundEngine.notifyListeners` batched per tick, drained by `haste.gui.SoundImpl` per frame).

Items 1-12 are the ones a player sees in normal survival within minutes. Items 3-11 share a
root cause (entities whose renderer is not a dumpable mob model), so a shared "special entity
renderer" table/registry entry pattern — like `DecorationKind` in
crates/haste-entity/src/registry.rs:925 — is the seam to extend, one module per vanilla
renderer class, which also keeps the Phase-2 mixin landmarks intact.
