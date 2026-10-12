# recorder-mod notes

## Build

26.3 ships Mojang names with no intermediary (see the spike REPORT.md), so the mod is built with
plain `javac` against `cache/server-spike/versions/26.3/server-26.3.jar` (the inner, de-bundled
server jar) plus sponge-mixin, fabric-loader, gson, authlib, brigadier and the whole
`cache/server-spike/libraries` tree — exactly the spike's `scripts/build-java.sh` approach, minus
Gradle/Loom. `build.sh` resolves the repo root from its own location and uses `cache/jdk`.

## Dump trigger

A mixin injects at `tickServer` TAIL and dumps once, then calls `server.halt(false)` so the dump
script terminates. First tick is late enough: `tickServer` only runs after `loadLevel()` has
finished (runServer calls it before the tick loop), so registries are booted and the overworld
exists (`server.overworld()` is used for the per-pose player dimensions).

## Decisions / caveats

- **Output dir** comes from the `-Dhaste.oracle.out=<dir>` system property, set by
  `dump-server-oracles.sh`. `MinecraftServer.getServerDirectory()` turned out to return the
  launch *working directory* (which for fabric-server-launch is the server dir, but deriving
  `cache/` from it via `getParent()` matched the wrong path component: it produced
  `oracle-server/oracle-dumps` instead of `cache/oracle`). The property is explicit and avoids
  guessing; the fallback is `<serverDir>/oracle-dumps/26.3`.
- **`fabric.mod.json` must list `"mixins": ["haste-recorder.mixins.json"]`** — the mod loaded
  fine without it but the mixin config was silently never applied (no error anywhere). First run
  produced no dumps for exactly this reason.
- **Shapes** are read via the state's own `getCollisionShape`/`getShape` with
  `EmptyBlockGetter.INSTANCE`, `BlockPos.ZERO`, `CollisionContext.empty()`: the no-neighbour
  shape. `toAabbs()` gives the box list; boxes are rounded to 1e-6.
- **`neighbours` flag** is a name heuristic (`*_wall`, `*_fence`, `*_fence_gate`, `*_pane`,
  `*_iron_bars`, `*_tripwire_hook`, `*_glass_pane`, `*_stained_glass_pane`, `*_door`, `*_trapdoor`,
  `*_gate`, plus `wall` and `mushroom_stem`): these blocks' shape methods read neighbouring
  blocks, so the empty-level shape may differ in world. I checked for a real runtime signal
  (`BlockBehaviour.Properties.dynamicShape` / `hasCollision`) but both are private/protected
  fields with only builder-style methods — no getters — so a heuristic is the honest option.
  14656 states / 121 blocks are flagged.
- **`opaque`** is `BlockStateBase.isSolidRender()` (no-arg in 26.3; the two-arg
  `isSolidRender(BlockGetter, BlockPos)` overload no longer exists on the state, only on
  BlockBehaviour with different arity semantics).
- **Player dimensions per pose**: constructs a real `ServerPlayer` in the overworld with a random
  UUID (never added to the world, just used for `getDimensions(Pose)`/`getEyeHeight(Pose)`) —
  this is the same path the game itself uses when it refreshes dimensions after a pose change.
- **`fluid`** is emitted only when the state has one (13389 states): `"fluid": "<id>"` plus
  `"fluid_empty": true|false` (fluid states of non-liquid blocks are usually empty).
- Deprecated-API warning at compile: `Block.stateById`/`Block.getId` are marked deprecated
  (they want you on the registry) but are exactly the BLOCK_STATE_REGISTRY view we need.

## Results (26.3)

35723 block states (matches datagen `blocks.json`), 161 entity types (matches datagen
`registries.json`), 1286 blocks. Sizes: block_shapes.json ~30.7 MB, entity_dimensions.json ~27 KB,
block_properties.json ~148 KB. Validation (script `validate-dumps.py`): stone full cube + opaque,
oak_slab bottom/top half, torch empty collision (outline 0.4375..0.5625 x/z, 0..0.625 y),
water empty collision + fluid tag, glowstone light 15, player standing 0.6x1.8 eye 1.62,
friction stone 0.6 / ice 0.98 / slime 0.8 / blue_ice 0.989.

## Dead ends

- Getting `dynamicShape` at runtime without the heuristic: no accessor exists (see above).
- Deriving the output dir from the server's own paths: `getServerDirectory()` is the CWD, not
  the install root; abandoned for the system property.

## Client recorder state (Task 3)

`src/main/java/haste/recorder/client/` (the `/haste record|screenshot|bench` commands,
scripted input sequences, trace/screenshot/bench writers, `MinecraftMixin` + `GuiMixin`)
compiles via `build.sh` — the jar contains both the server dumps and the client recorder,
and `fabric.mod.json` is now `environment: "*"` with a `client` entrypoint. It has **not
been run against the real client yet** (needs a display + the user; exact steps in
`docs/RECORDING.md`). `build.sh` additionally requires `cache/versions/26.3/client.jar`
and unpacks three Fabric API nested jars (base, command-api, lifecycle-events) for javac.
`validate-traces.py` sanity-checks recorded traces once they exist.
