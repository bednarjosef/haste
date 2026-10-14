# Crate interfaces (the contract between workstreams)

Owned by the lead. Signatures here are the agreement; implementers may add methods but not change
or remove these without asking. Types without a crate prefix are from `haste_core`.

## haste-data (world-and-data)

```rust
pub struct GameData { pub blocks: BlockStates, pub registries: Registries, pub shapes: BlockShapes }
impl GameData {
    /// Loads `datagen/26.3/reports/{blocks,registries}.json` and `oracle/26.3/block_shapes.json`
    /// (optional, fallback shapes otherwise) from `haste_core::paths`. ~100 ms.
    pub fn load() -> anyhow::Result<GameData>;
    pub fn load_from(reports: &Path, oracle: Option<&Path>) -> anyhow::Result<GameData>;
}
pub struct BlockStates { .. }
impl BlockStates {
    pub fn len(&self) -> usize;
    pub fn block_of(&self, s: BlockStateId) -> &Identifier;
    pub fn properties(&self, s: BlockStateId) -> Vec<(String, String)>;   // sorted by name (interned storage; owned result)
    pub fn property_value(&self, s: BlockStateId, name: &str) -> Option<&str>;   // hot-path lookup
    pub fn default_state(&self, block: &Identifier) -> Option<BlockStateId>;
    pub fn state(&self, block: &Identifier, props: &[(&str, &str)]) -> Option<BlockStateId>;
    pub fn with_property(&self, s: BlockStateId, name: &str, value: &str) -> Option<BlockStateId>;
    pub fn block_index(&self, block: &Identifier) -> Option<RegistryIndex>;    // minecraft:block registry
}
pub struct Registries { .. }
impl Registries {
    pub fn index(&self, registry: &str, entry: &Identifier) -> Option<RegistryIndex>;   // e.g. ("minecraft:entity_type", "minecraft:zombie")
    pub fn name(&self, registry: &str, i: RegistryIndex) -> Option<&Identifier>;
    pub fn len(&self, registry: &str) -> usize;
}
pub struct BlockShapes { .. }
impl haste_core::view::CollisionShapes for BlockShapes { .. }
impl haste_core::view::BlockPhysics for GameData { .. }      // friction/speed/jump from block_properties.json; fluid/light/air/opaque from block_shapes.json
impl haste_core::view::BlockStateTable for BlockStates { .. }
impl BlockShapes { pub fn is_oracle_backed(&self) -> bool; }
```
Fallback shapes (no oracle file): full cube for every state except air/cave_air/void_air, fluids,
and blocks whose path contains one of: grass, fern, flower, sapling, torch, rail, button, lever,
pressure_plate, carpet, sign, banner, vine, bush, mushroom, wheat, carrot, potato, beetroot,
sugar_cane, kelp, seagrass, lily, dead_bush, cobweb, redstone_wire, tripwire, string, cave_vines,
pointed_dripstone(half), snow (height 1/8 per layer property), slab (half), stairs (half cube).
Document the fallback as approximate; the oracle file replaces it.

## haste-protocol (protocol)

```rust
pub enum Account { Offline { username: String }, Microsoft(MsaTokens) }
pub struct ConnectOptions { pub address: String, pub account: Account, pub protocol_version: i32 /* 26.3 */ }
pub struct Connection { .. }   // owned by a tokio task; game thread talks through channels
impl Connection {
    /// Resolves SRV, handshakes, logs in (encryption + Mojang session for online mode), runs the
    /// configuration state (known packs, registry data, finish) and returns in the play state.
    pub async fn connect(opts: ConnectOptions) -> Result<(Connection, Login), ProtocolError>;
    pub fn try_recv(&mut self) -> Option<ClientboundPlay>;     // non-blocking, for the game tick
    pub async fn recv(&mut self) -> Option<ClientboundPlay>;
    pub fn send(&self, p: ServerboundPlay) -> Result<(), ProtocolError>;
    pub fn registries(&self) -> &SyncedRegistries;             // from the configuration state
    pub fn disconnect(self);
}
pub struct Login { pub entity_id: EntityId, pub dimension: haste_core::game::DimensionInfo, pub game_mode: GameMode, pub view_distance: i32, pub simulation_distance: i32, pub hardcore: bool, .. }
pub struct SyncedRegistries { .. }  // registry_data packets: dimension types, biomes, damage types, ... by Identifier, raw NBT kept
pub enum ClientboundPlay { /* one variant per packet in packets.json play.clientbound, named like the packet: LevelChunkWithLight(LevelChunkWithLight), SetEntityData(SetEntityData), ... Unknown { id: i32, body: Bytes } */ }
pub enum ServerboundPlay { /* same for play.serverbound */ }
```
Packet structs use `haste_core` types where they exist: chunk packets produce
`haste_core::chunk::ChunkData` / `LightData`; `add_entity` produces `haste_core::entity::EntitySpawn`;
entity metadata decodes to `Vec<MetadataEntry>`; text is `Component` (JSON and NBT encodings);
positions are `BlockPos` (packed long) / `DVec3`. Packet ids are loaded from
`cache/datagen/26.3/reports/packets.json` at startup (`PacketIds::load()`), never hard-coded.
Keep-alive is answered inside the connection task. Unknown packets are delivered as `Unknown` and
counted, never dropped silently.

## haste-world (world-and-data)

```rust
pub struct World { .. }
impl World {
    pub fn new(dim: DimensionInfo, data: Arc<GameData>) -> World;
    pub fn load_chunk(&mut self, data: &haste_core::chunk::ChunkData) -> Result<(), WorldError>;
    pub fn unload_chunk(&mut self, pos: ChunkPos);
    pub fn set_block(&mut self, pos: BlockPos, state: BlockStateId);
    pub fn apply_light(&mut self, pos: ChunkPos, light: &LightData);
    pub fn chunk(&self, pos: ChunkPos) -> Option<&Chunk>;
    pub fn loaded_chunks(&self) -> impl Iterator<Item = ChunkPos>;
    pub fn snapshot_section(&self, pos: SectionPos) -> Option<Arc<SectionSnapshot>>;  // immutable copy for meshing threads; with its 26 neighbours' border data available through `SectionView`
}
impl haste_core::view::BlockView for World {}
impl haste_core::view::DirtyTracker for World {}
// Block entities (`crates/haste-world/src/world.rs`): one per loaded chunk, keyed by BlockPos.
impl World {
    pub fn block_entity(&self, pos: BlockPos) -> Option<&BlockEntityData>;                     // None when the chunk is not loaded
    pub fn block_entities_in(&self, pos: ChunkPos) -> impl Iterator<Item = (BlockPos, &BlockEntityData)>;   // arbitrary order
    pub fn set_block_entity(&mut self, pos: BlockPos, data: BlockEntityData);                   // ignored if the chunk is not loaded; marks the section dirty
}
// Dirty rules: `set_block` marks every section whose one-block border touches the block (the
// 3x3x3 block neighbourhood: {0} per axis plus {-1} on the low and {+1} on the high border), so
// diagonally adjacent sections remesh too (AO and smooth light sample that neighbourhood);
// sections outside the dimension's vertical span are skipped. `set_block_entity` marks only the
// block's own section; `apply_light` marks the 3x3x3 section neighbourhood of every changed light
// section, likewise. `load_chunk` dirties the chunk's sections plus its neighbours' borders.
pub struct Chunk { .. }  impl Chunk { pub fn section(&self, y: i32) -> Option<&Section>; pub fn heightmap(..) }
pub struct Section { .. } impl Section { pub fn get(&self, x: u8, y: u8, z: u8) -> BlockStateId; pub fn is_empty(&self) -> bool; pub fn biome(&self, ..) -> BiomeId }
pub struct PalettedContainer<T> { .. }   // vanilla wire format: single value / indirect / direct
pub struct SectionSnapshot { .. }  impl SectionSnapshot + SectionView: get(x,y,z) incl. neighbours in -1..=16, sky/block light likewise
```

## haste-assets (renderer)

```rust
pub struct Assets { .. }
impl Assets {
    pub fn open(client_jar: &Path, assets_dir: &Path, assets_index: &str) -> anyhow::Result<Assets>;
    pub fn texture(&self, id: &Identifier) -> anyhow::Result<image::RgbaImage>;  // textures/<path>.png
    pub fn blockstate(&self, block: &Identifier) -> anyhow::Result<BlockStateDefinition>;  // blockstates/<path>.json (variants + multipart)
    pub fn model(&self, id: &Identifier) -> anyhow::Result<ModelJson>;             // models/<path>.json with parent resolution
    pub fn language(&self, code: &str) -> anyhow::Result<HashMap<String, String>>;  // lang/en_us.json from jar (+ assets index for others)
    pub fn font_default(&self) -> anyhow::Result<BitmapFont>;                       // font/default.json bitmap providers -> glyph atlas
    pub fn gui_sprite(&self, id: &Identifier) -> anyhow::Result<(image::RgbaImage, SpriteMeta)>;
}
pub struct BakedBlockModels { .. }  // per BlockStateId: list of quads (positions, uv in atlas, normal/face, cull face, tint index, ambient occlusion flag)
impl BakedBlockModels { pub fn bake(assets: &Assets, states: &dyn haste_core::view::BlockStateTable, atlas: &mut AtlasBuilder) -> anyhow::Result<Self>; pub fn quads(&self, s: BlockStateId) -> &[BakedQuad]; pub fn is_full_opaque(&self, s: BlockStateId) -> bool; pub fn render_layer(&self, s: BlockStateId) -> RenderLayer }
// `haste_data::BlockStates` implements `BlockStateTable`; the atlas is built during baking (textures are discovered then).
// 26.3 facts (renderer): the jar carries the full asset tree (no extracted objects dir needed); blockstate variant keys are partial (missing properties are wildcards, most specific wins); models have no `render_type`, the layer is a documented heuristic.
// Animated sprites (`crates/haste-assets/src/assets.rs`, `atlas.rs`): the atlas keeps the animated
// textures as `AnimatedSprite`s; the renderer re-uploads each sprite's region every game tick.
pub struct AnimationSchedule { pub frames: Vec<AnimationFrame>, pub interpolate: bool }
impl AnimationSchedule { pub fn sequential(frame_count: u32, frametime: u32, interpolate: bool) -> Self; pub fn total_ticks(&self) -> u64 }
pub fn frame_at(schedule: &AnimationSchedule, tick: u64) -> (u32, u32, f32);  // current sheet frame, next frame, blend towards it (0 without `interpolate`)
pub struct AnimatedSprite { pub name: String, pub region: AtlasRegion, pub frames: Vec<image::RgbaImage>, pub schedule: AnimationSchedule }
impl AnimatedSprite { pub fn initial_frame(&self) -> &image::RgbaImage; pub fn frame_pixels(&self, tick: u64) -> Cow<'_, [u8]> }   // Cow::Borrowed unless the schedule interpolates
impl Assets { pub fn animation(&mut self, id: &Identifier) -> Option<Animation>; pub fn texture_first_frame(&mut self, id: &Identifier) -> Result<image::RgbaImage> }
// Particles (`crates/haste-assets/src/particle.rs`): a particle type's json lists its sprites;
// block/item types have no json and resolve to `None`. One atlas holds every referenced sprite.
impl Assets { pub fn particle_definition(&mut self, id: &Identifier) -> Result<Option<ParticleDefinition>> }
impl ParticleAtlas { pub fn build(assets: &mut Assets, particle_ids: &[Identifier]) -> Result<ParticleAtlas>; pub fn region_of(&self, texture_id: &Identifier) -> Option<AtlasRegion>; pub fn sprites_of(&self, particle: &Identifier) -> &[AtlasRegion]; pub fn pixels(&self) -> &[u8]; pub fn size(&self) -> u32 }
pub fn mean_linear(pixels: [[u8; 4]; 4]) -> [u8; 4];   // sRGB-correct 2x2 average (Pomme), also used for the atlas mip chain
```

## haste-render (renderer)

```rust
pub struct Renderer { .. }
impl Renderer {
    pub fn new(window: Arc<winit::window::Window>, assets: &Assets, data: Arc<GameData>) -> anyhow::Result<Renderer>;
    pub fn resize(&mut self, w: u32, h: u32);
    /// Submits dirty sections to the mesher threads (takes snapshots from the world) and uploads finished meshes.
    pub fn update_world(&mut self, world: &World, dirty: &[SectionPos], camera_chunk: ChunkPos, view_distance: i32);
    pub fn render(&mut self, frame: &FrameInput) -> anyhow::Result<FrameStats>;
    pub fn text_metrics(&self) -> Arc<dyn haste_core::gui::TextMetrics>;
}
pub struct FrameInput<'a> { pub camera: Camera, pub entities: &'a [haste_core::model::EntityInstance], pub gui: &'a haste_core::gui::GuiFrame, pub sky: SkyState, pub partial_tick: f32,
    /// Block under the crosshair and the face hit: drawn as the outline of its outline shape.
    pub block_target: Option<(BlockPos, Direction)>,
    /// Crack overlays: our own break progress plus other players' `block_destruction` stages (0..=9).
    pub breaking: &'a [(BlockPos, u8)] }
pub struct Camera { pub pos: DVec3, pub yaw: f32, pub pitch: f32, pub fov_deg: f32, pub roll_deg: f32 /* hurt tilt, degrees */ }
pub struct SkyState { pub time_of_day: i64, pub rain: f32, pub dimension_has_sky: bool, pub fog_start: f32, pub fog_end: f32 }
pub struct FrameStats { pub frame_ms: f32, pub chunk_sections_drawn: u32, pub chunk_sections_total: u32, pub meshes_pending: u32, pub draw_calls: u32, pub uploads: u32, pub upload_bytes: u64 }
```
Meshing runs on a thread pool from `SectionSnapshot`s; the render thread never blocks on it.

## haste-entity (entities)

```rust
pub struct EntityStore { .. }
impl EntityStore {
    pub fn new(data: Arc<GameData>) -> EntityStore;   // interim on the branch: new(dims: EntityDimensions) until haste-data lands
    pub fn spawn(&mut self, s: &haste_core::entity::EntitySpawn);
    pub fn remove(&mut self, ids: &[EntityId]);
    pub fn move_delta(&mut self, id: EntityId, delta: DVec3, on_ground: bool);           // short deltas /4096
    pub fn set_rotation(&mut self, id: EntityId, yaw: f32, pitch: f32, on_ground: bool);
    pub fn teleport(&mut self, id: EntityId, pos: DVec3, yaw: f32, pitch: f32, on_ground: bool);
    pub fn set_head_yaw(&mut self, id: EntityId, yaw: f32);
    pub fn set_velocity(&mut self, id: EntityId, v: DVec3);
    pub fn set_metadata(&mut self, id: EntityId, entries: &[MetadataEntry]);
    pub fn set_equipment(&mut self, id: EntityId, slot: u8, item: ItemStackData);
    pub fn player_info(&mut self, info: &[PlayerInfo]);   // for skins/nametags
    pub fn tick(&mut self);                                 // 20 Hz: advance interpolation targets
    pub fn get(&self, id: EntityId) -> Option<&Entity>;
    pub fn render_instances(&self, partial_tick: f32, out: &mut Vec<haste_core::model::EntityInstance>);  // interpolated
    pub fn len(&self) -> usize;
}
pub struct Entity { pub id, pub uuid, pub kind: RegistryIndex, pub pos: DVec3, pub prev_pos: DVec3, pub yaw, pub pitch, pub head_yaw, pub on_ground, pub pose: Pose, pub dimensions: (f32, f32) /* width, height from oracle or default */, .. }
```
Models (`haste_core::model::CubeModel`) are defined in this crate as data (player/humanoid, quadruped, generic box) and chosen by entity type.

## haste-physics (physics)

```rust
pub struct PlayerState { pub pos: DVec3, pub velocity: DVec3, pub yaw: f32, pub pitch: f32, pub on_ground: bool, pub sprinting: bool, pub sneaking: bool, pub in_water: bool, pub flying: bool, pub fall_distance: f32, .. }
pub struct PlayerAttributes { pub movement_speed: f64 /* 0.1 */, pub jump_strength: f64, pub gravity: f64, pub step_height: f64, pub width: f32, pub height: f32 /* standing/sneaking/swimming via pose */ }
pub fn tick(state: &mut PlayerState, input: &InputState, attrs: &PlayerAttributes, world: &dyn BlockView, shapes: &dyn CollisionShapes, blocks: &dyn haste_core::view::BlockPhysics) -> TickResult;
pub struct TickResult { pub moved: bool, pub rotated: bool, pub on_ground_changed: bool, pub collided_horizontally: bool }
pub fn collide(aabb: Aabb, movement: DVec3, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> DVec3;   // pure axis sweep, tested in isolation
pub fn collide_with_step(aabb: Aabb, movement: DVec3, step_height: f64, on_ground: bool, world: &dyn BlockView, shapes: &dyn CollisionShapes) -> DVec3;
// PlayerState also carries: in_lava, swimming, pose: Pose, jump_cooldown: u8, horizontal_collision, vertical_collision.
// PlayerAttributes also carries: sneaking_speed, flying_speed: f32, crouching_height, swimming_height.
// `haste_data::GameData` implements `BlockPhysics` (from block_properties.json + block_shapes.json).
```
Validated tick by tick against traces in `cache/oracle/26.3/traces/*.json` (format in haste-testkit).

## haste-jvm (jvm-host)

```rust
pub struct JvmHost { .. }
impl JvmHost {
    /// Starts the JVM with: tier-3 class loader over the player's client jar (allow-list of packages/classes that are stateless), generated proxies for tier-1 types (allow-list), haste runtime classes. Never the whole jar on the classpath.
    pub fn start(cfg: JvmConfig) -> anyhow::Result<JvmHost>;
    pub fn gui(&self) -> &GuiPipeline;
}
pub struct GuiPipeline { .. }
impl GuiPipeline {
    pub fn open_screen(&self, screen: ScreenKind) -> anyhow::Result<()>;   // Chat, Pause, Inventory (real vanilla classes)
    pub fn close_screen(&self);
    pub fn is_open(&self) -> bool;
    pub fn key(&self, key: i32, keycode: i32, mods: i32, pressed: bool) -> bool;   // 26.3 uses SDL3: key = scancode, keycode = SDL keycode (EditBox matches on keycode), mods = SDL keymod; returns consumed
    pub fn char_typed(&self, c: char, mods: i32) -> bool;
    pub fn mouse(&self, x: f64, y: f64, button: Option<(i32, bool)>) -> bool;
    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64) -> bool;
    /// Runs the screen's extractRenderState into the batch and returns the frame: ONE JNI call + one array drain.
    pub fn render(&self, width: u32, height: u32, scale: u32, mouse: (f64, f64), partial: f32) -> anyhow::Result<haste_core::gui::GuiFrame>;
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>);   // Font.width natives call back here
}
```
`JvmConfig` and `ScreenKind` are defined in `haste-jvm`. Extra methods on the implementation: `tick`, `screen_class`, `mouse_grabbed`, `set_mouse_grabbed`, `take_last_error`, `with_state` (Rust-owned `GameState`: inventory slots, selected slot, chat lines, game mode; screens write back sent chat/commands and container clicks).

Tier-2 proxies in Phase 1: `Minecraft` (getInstance, player, level, options, font, screen, setScreen), `LocalPlayer`/`Player`/`Inventory`/`ItemStack` (what the inventory screen reads), `Font`, `GuiGraphicsExtractor` (batched delegates), `Window`, `Options`, `KeyMapping`, `SoundManager` (no-op). Everything else: real vanilla classes or the Unimplemented log.

## haste-setup (setup)

```rust
pub struct Installation { pub client_jar: PathBuf, pub server_jar: PathBuf, pub assets_dir: PathBuf, pub assets_index: String, pub libraries_dir: PathBuf, pub jdk_java: PathBuf, pub reports_dir: PathBuf }
pub async fn ensure_installed(progress: impl FnMut(Progress) + Send + 'static) -> anyhow::Result<Installation>;  // manifest -> version json -> client/server jars (sha1), assets index + objects, libraries needed to run the vanilla server + client, JDK 25 (Adoptium) if no suitable java, then data generator reports
pub fn find_existing_minecraft() -> Option<PathBuf>;   // ~/.minecraft/versions/26.3 etc. (read only, never modified)
pub struct LocalServer { .. }  impl LocalServer { pub async fn start(world_dir: &Path, port: u16) -> anyhow::Result<LocalServer>; pub fn address(&self) -> String; pub async fn stop(self); }  // vanilla server jar, offline mode, 127.0.0.1, waits for "Done"
// Config and options (`crates/haste-setup/src/config.rs`, `options.rs`): TOML in the platform
// config dir, written 0600/0700 because of the tokens.
pub struct Config { pub username: String, pub render_distance: i32, pub gui_scale: u32, pub fov: f32, pub last_server: Option<String>, pub msa: Option<MsaTokens>, pub player_certificates: Option<PlayerCertificates>, pub options: Options }
impl Config { pub fn load() -> anyhow::Result<Config>; pub fn load_from(path: &Path) -> anyhow::Result<Config>; pub fn save(&self) -> anyhow::Result<()>; pub fn save_to(&self, path: &Path) -> anyhow::Result<()> }   // missing file = defaults written; options sanitized on load
pub struct Options { pub render_distance: u8, pub simulation_distance: u8, pub fov: u8, pub mouse_sensitivity: f32, pub gui_scale: u8, pub fullscreen: bool, pub vsync: bool, pub max_fps: u32, pub sound: SoundVolumes, pub keys: Keybinds }
impl Options { pub fn sanitized(self) -> Options; pub fn import_vanilla(options_txt: &str) -> Options }   // vanilla options.txt (key:value); unknown keys ignored, ranges clamped
impl Keybinds { pub fn vanilla() -> Keybinds; pub fn get(&self, action: &str) -> Option<&str> }   // "key.forward" -> "key.keyboard.w"
// MSA (`crates/haste-setup/src/msa.rs`): the whole device-code flow is async over a
// `reqwest::Client`; `now` is a parameter so refresh decisions are testable.
pub async fn device_code_login<F: FnOnce(&DeviceCode)>(on_code: F) -> anyhow::Result<MsaTokens>;   // begin -> poll (interval, slow_down) -> XBL -> XSTS -> MC login -> profile
pub async fn begin_device_code(client: &reqwest::Client) -> anyhow::Result<DeviceCode>;
pub async fn poll_device_code(client: &reqwest::Client, device_code: &DeviceCode) -> anyhow::Result<Option<OauthToken>>;   // None = still pending
pub async fn player_certificates(client: &reqwest::Client, mc_access_token: &str) -> anyhow::Result<PlayerCertificates>;   // chat-signing key pair
pub async fn ensure_fresh_account(client: &reqwest::Client, config: &mut Config) -> anyhow::Result<()>;   // refreshes the MC token when expired, refetches certificates, saves the config
pub fn mc_token_needs_refresh(tokens: &MsaTokens, now: i64) -> bool;
pub fn certificates_need_refresh(certs: &Option<PlayerCertificates>, now: i64) -> bool;
```

## haste-testkit (oracles)

Formats (JSON, versioned, documented in `crates/haste-testkit/FORMATS.md`): `block_shapes.json`
(state id -> collision + outline boxes), `entity_dimensions.json`, `traces/<name>.json` (per tick:
input, pos, velocity, on_ground, pose, from the recorder mod), `screenshots/<name>.png` + camera
JSON, `packets/<name>.bin` (length-prefixed clientbound play packets as received). Harness:
`replay_trace(trace, tick_fn) -> Report` (per-tick position error, first divergence),
`compare_screenshot(a, b) -> Diff` (percentage of differing pixels with tolerance), `Bench` runner
writing JSON + Markdown tables.

## haste-audio (sound)

`crates/haste-audio/src`: `sounds.rs` (parsing and variant picking), `engine.rs` (the tracking
half), `output.rs` (rodio worker + `SoundSystem`), `spatial.rs` (math), `music.rs` (situation
rules). Everything is tested without a device; the silent fallback never fails.

```rust
// sounds.rs: `sounds.json` from the assets; keys are namespaced event names.
pub struct SoundEvents { .. }   // "minecraft:block.stone.break"
impl SoundEvents {
    pub fn parse(json: &[u8]) -> anyhow::Result<SoundEvents>;
    pub fn get(&self, event: &str) -> Option<&SoundEvent>;
    pub fn subtitle(&self, event: &str) -> Option<&str>;
    pub fn pick(&self, event: &str, seed: i64) -> Option<SoundVariant>;   // weighted, deterministic (our own RNG, not vanilla's sequence); follows `type: "event"` up to 8 levels
}
// engine.rs: no output at all, only what plays and how loud it is.
pub enum SoundSource { Master, Music, Record, Weather, Block, Hostile, Neutral, Player, Ambient, Voice, Ui }
impl SourceVolumes { pub fn get(&self, source: SoundSource) -> f32; pub fn set(&mut self, source: SoundSource, volume: f32); pub fn effective(&self, source: SoundSource) -> f32 }   // master * source
impl SilentEngine {   // Default
    pub fn new() -> Self;
    pub fn play(&mut self, variant: &SoundVariant, source: SoundSource, pos: Option<DVec3>, volume: f32, pitch: f32, event: Arc<str>);   // pos None = global, centred; event = the sound event it plays
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32);   // vanilla yaw: 0 = +Z, clockwise from above
    pub fn set_volume(&mut self, source: SoundSource, volume: f32);
    pub fn stop(&mut self, source: Option<SoundSource>, event: Option<&str>);   // by sound event name (namespace optional), None on an axis matches everything (vanilla SoundEngine.stop)
    pub fn remove(&mut self, index: usize);                    // for finished sounds, like Vec::remove
    pub fn tick(&mut self);                                    // recomputes gains from the listener and volumes
    pub fn playing(&self) -> usize; pub fn playing_in(&self, source: SoundSource) -> usize; pub fn is_event_playing(&self, event: &str) -> bool;
    pub fn gains(&self) -> impl Iterator<Item = (f32, f32)>; pub fn pitches(&self) -> impl Iterator<Item = f32>   // in play order
}
// spatial.rs: pure math.
pub fn attenuation(distance: f64, volume: f32, attenuation_distance: u32) -> f32;   // linear falloff to 0 at distance * max(volume, 1)
pub fn pan(listener_pos: DVec3, listener_yaw_deg: f32, source: DVec3) -> f32;        // -1 left ..= +1 right
pub fn stereo_gains(gain: f32, pan: f32) -> (f32, f32);                              // constant power
pub fn clamp_pitch(p: f32) -> f32;                                                   // 0.5..=2.0
// output.rs: the device and its decode worker, plus the engine + assets facade.
pub struct AudioOutput { .. }   // no channel = silent fallback, logged once; `open` never fails
impl AudioOutput { pub fn open() -> AudioOutput; pub fn silent() -> AudioOutput; pub fn is_live(&self) -> bool }   // silent: never opens a device (tests)
impl SoundSystem {
    pub fn new(events: SoundEvents, assets: haste_assets::Assets) -> Self;   // opens the default device, or stays silent
    pub fn with_output(events: SoundEvents, assets: haste_assets::Assets, output: AudioOutput) -> Self;
    pub fn play(&mut self, event: &str, source: SoundSource, pos: Option<DVec3>, volume: f32, pitch: f32, seed: i64);   // unknown event = nothing
    pub fn play_variant(&mut self, variant: &SoundVariant, source: SoundSource, pos: Option<DVec3>, volume: f32, pitch: f32, event: &str);   // event names interned once
    pub fn stop(&mut self, source: Option<SoundSource>, event: Option<&str>);   // like SilentEngine::stop
    pub fn set_listener(&mut self, pos: DVec3, yaw_deg: f32); pub fn set_volume(&mut self, source: SoundSource, volume: f32);
    pub fn tick(&mut self);                                                   // 20 Hz, no allocation when nothing started
    pub fn playing(&self) -> usize; pub fn playing_in(&self, source: SoundSource) -> usize; pub fn is_event_playing(&self, event: &str) -> bool;
}
// music.rs: decides when background music plays, owns no output.
pub enum MusicSituation { Menu, Game, Creative, UnderWater, End, Dragon, Credits, Biome { event: String, min_delay: u32, max_delay: u32, replace: bool } }
impl MusicSituation { pub fn music(&self) -> MusicSpec }                       // event + vanilla's delays
pub enum MusicCommand { Play(String), Stop }
pub struct MusicManager { .. }   // Default
impl MusicManager { pub fn new() -> Self; pub fn set_situation(&mut self, situation: MusicSituation); pub fn situation(&self) -> &MusicSituation; pub fn playing_event(&self) -> Option<&str>; pub fn tick(&mut self, rng: &mut MusicRng, is_playing: bool) -> Option<MusicCommand> }
pub struct MusicRng { .. }  impl MusicRng { pub fn new(seed: u64) -> Self; pub fn range(&mut self, min: u32, max: u32) -> u32 }   // xorshift, replays for a fixed seed
```

## haste-client player (player): `crates/haste-client/src/player.rs`

```rust
pub struct LocalPlayer { pub state: haste_physics::PlayerState, pub attrs: haste_physics::PlayerAttributes, .. }
impl LocalPlayer {
    pub fn new(login: &haste_protocol::Login) -> LocalPlayer;
    /// Consumes the packets about the local player (teleports, abilities, attributes, motion,
    /// respawn, health, game mode, held slot). Returns false for packets it does not handle.
    pub fn handle(&mut self, p: &haste_protocol::ClientboundPlay, out: &mut Vec<haste_protocol::ServerboundPlay>) -> bool;
    /// One 20 Hz tick: physics with this tick's input, then the serverbound packets a vanilla
    /// client sends for it, in vanilla's order (player_input, player_command, move_player_*,
    /// ..., client_tick_end last).
    pub fn tick(&mut self, input: &haste_core::input::InputState, world: &haste_world::World, data: &haste_data::GameData, out: &mut Vec<haste_protocol::ServerboundPlay>);
    /// Breaking, placing, using, attacking, picking and hotbar changes for this tick
    /// (`haste_core::input::InteractInput`); runs inside `tick` in vanilla's order.
    pub fn set_interact(&mut self, input: haste_core::input::InteractInput);
    /// Block under the crosshair and the face hit; our own break progress (stage 0..=9).
    pub fn target(&self) -> Option<(BlockPos, Direction)>;
    pub fn break_progress(&self) -> Option<(BlockPos, u8)>;
    /// Eye position and look angles interpolated by partial tick, for the camera.
    pub fn camera(&self, partial_tick: f32) -> (glam::DVec3, f32, f32);
}
```

## haste-client ui (jvm-host): `crates/haste-client/src/ui.rs`

HUD (vanilla `Hud`) and screens in one JNI call per frame. Not yet wired into game.rs/app.rs.

```rust
#[derive(Clone)] pub struct Ui { .. }   // one clone per thread; JVM calls are serialised inside
impl Ui {
    pub fn new(host: JvmHost) -> Ui;                       // JvmHost::start takes 9-47 s: own thread
    pub fn set_text_metrics(&self, m: Arc<dyn TextMetrics>);
    pub fn joined(&self, conn: &Connection, login: &Login) -> anyhow::Result<()>;
    pub fn handle(&self, p: &ClientboundPlay);             // game thread: every packet, in order
    pub fn tick(&self);                                    // 20 Hz
    pub fn drain_outgoing(&self, out: &mut Vec<ServerboundPlay>);  // once per tick
    pub fn key(&self, key: i32, keycode: i32, mods: i32, pressed: bool) -> bool;  // false = game input
    pub fn char_typed(&self, c: char, mods: i32) -> bool;
    pub fn mouse_moved(&self, x: f64, y: f64) -> bool;    // GUI-scaled pixels
    pub fn mouse_button(&self, x: f64, y: f64, button: i32, pressed: bool) -> bool;  // 1 left, 2 middle, 3 right
    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64) -> bool;
    pub fn frame(&mut self, w: u32, h: u32, scale: u32, mouse: (f64, f64), partial: f32) -> &GuiFrame;
    pub fn screen_open(&self) -> bool;
    pub fn wants_mouse_grabbed(&self) -> bool;
    pub fn set_mouse_grabbed(&self, g: bool);
    pub fn selected_slot(&self) -> i32;
    pub fn take_disconnect_request(&self) -> bool;
    pub fn disconnected(&self, reason: &Component);
    pub fn set_debug(&self, d: haste_jvm::DebugInfo);
    pub fn set_eye_in_water(&self, v: bool);
}
```
With no screen open, `key` handles T, /, E, Escape, F1, F3, Tab and 1-9 like vanilla.

## haste-client CLI (`crates/haste-client/src/main.rs`, `launch.rs`)

```rust
haste [--server HOST[:PORT]] [--username NAME] [--headless] [--seconds N] [--ping] [--screenshot PNG]
      [--bench N [--bench-out JSON]] [--render-distance N] [--width N --height N] [--yaw D --pitch D]
      [--pos X Y Z] [--perspective first|back|front] [--vsync BOOL] [--window-seconds N]
      <setup|singleplayer|login|logout|vanilla>
haste setup                                                    // idempotent: jars, assets, libraries, JDK, datagen reports
haste singleplayer [--world NAME] [--no-window] [--window-seconds N]   // embedded vanilla server + the client joined to it
haste login                                                     // Microsoft device-code flow, stores the account in the config
haste logout                                                    // removes the stored account
haste vanilla [--fabric] [--server HOST:PORT] [--username NAME]  // launches the real 26.3 client for comparison
```
Launch stops after the subcommand it was given (`launch::run`). `--bench` and `--screenshot`
need `--server` and render offscreen; `--bench` walks a fixed camera path for N seconds and
prints the frame-time summary (`--bench-out` writes it as JSON). `--render-distance` and
`--vsync` override the saved option for that run only; `--perspective` selects the screenshot
camera (first person, third person back, front looking back at the player). The windowed
launcher flow (no subcommand) opens the game window and joins `--server` or the last one.
Unattended runs use `--headless`, `--screenshot`, `--bench` or `--window-seconds`.

## haste-render additions (session 2)

`Renderer`: `text_metrics()`, `set_item_registry(&Registries)`, `sample_world(world, shapes,
&FrameInput)` and `sample_entity_lights(world, entities)` (the renderer holds no world; call
before `render`), `set_dimension(&DimensionInfo)`, `set_first_person(Option<FirstPerson>)`,
`set_vsync(bool)`, `update_world_from(&Arc<dyn SnapshotSource>, dirty, chunk, vd)` (meshing
capture on worker threads), `timings() -> FrameTimings`, `gui_stats()`. Free:
`haste_render::default_skin(uuid) -> (Identifier, slim)`. `WGPU_BACKEND` selects the backend.

## haste-render additions (animated sprites, capture, adapter, stats)

`crates/haste-render/src/renderer.rs`: `set_animation_tick(tick: u64)` (game tick; the next
`render` re-uploads every changed atlas sprite region), `adapter_info() -> wgpu::AdapterInfo`
(name/backend for bench reports), `request_capture()` / `take_capture() -> Option<Result<RgbaImage>>`
(F2: non-blocking readback, `take_capture` pumps the map callback and returns `None` while
pending; the offscreen path uses `read_back` instead). `Camera.roll_deg` carries the hurt tilt.
`GuiBlend` (`haste_core::gui`): `Normal` (alpha), `Multiply` (vignette), `Invert` (crosshair);
the renderer keeps one pipeline per variant. `FrameStats` also reports `uploads` (section meshes
written this frame) and `upload_bytes` (vertex+index bytes).

## haste-client wiring (lead)

The game thread runs at 20 Hz: drain `Connection::try_recv` -> apply to `World`/`EntityStore`/
player; `physics::tick` with the sampled `InputState`; send movement packets; `EntityStore::tick`.
The render thread (winit event loop) builds `FrameInput` from a shared snapshot (interpolation by
partial tick) and calls `Renderer::render`. `GuiFrame` comes from the HUD (Rust) plus
`GuiPipeline::render` when a screen is open.
