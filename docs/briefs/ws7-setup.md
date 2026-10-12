# WS7: haste-setup (Mojang downloads, data generator, config, local server, launcher flow)

You own `crates/haste-setup` and later `crates/haste-client/src/launch.rs` (CLI/config wiring;
wait for the lead's go). Interfaces in docs/INTERFACES.md. Spike scripts to learn from (bash, in
`../haste-spike/scripts/setup.sh` and `server.sh`): manifest ->
version JSON -> jars with sha1 check, Temurin JDK via Adoptium API, Fabric installer, server
properties for offline/localhost, data generator run (`-DbundlerMainClass=net.minecraft.data.Main
-jar server.jar --reports --all`).

## Task 1: `ensure_installed`
- Mojang official endpoints only: `https://piston-meta.mojang.com/mc/game/version_manifest_v2.json`,
  the 26.3 version JSON, `downloads.client/server` (sha1 verified), `assetIndex` -> objects from
  `https://resources.download.minecraft.net/<2>/<hash>` (parallel downloads, resume, sha1),
  `libraries` for this OS (with natives classifiers if present) into `cache/libraries` in Maven
  layout, log4j config. Layout per `haste_core::paths`. Idempotent and resumable; a progress
  callback with bytes/total; concurrency limited (8).
- JDK: use `cache/jdk` if present, else an installed `java` >= 25 found on PATH/JAVA_HOME, else
  download Temurin 25 (Adoptium API) into `cache/jdk`. Never install system-wide.
- Data generator: run the server jar as above into `cache/datagen/26.3` (skip if reports exist).
- `find_existing_minecraft()`: detect `~/.minecraft` (Linux/macOS/Windows paths) with a 26.3
  version; read-only (we may reuse its jar/assets to avoid downloads; never write there).
- Tests: unit tests for manifest/version JSON parsing on small fixtures (write your own fixtures;
  do not commit Mojang JSON verbatim beyond a few fields), sha1 verification, Maven path
  derivation, asset object path derivation. An `#[ignore]` integration test runs
  `ensure_installed` against the cache (it should be a no-op if everything is there).

## Task 2: config + local server + launcher flow
- `Config` TOML in the platform config dir (`directories`): username, render distance, gui scale,
  fov, last server, MSA tokens (encrypted at rest is out of scope; mark the file 0600).
- `LocalServer::start(world_dir, port)`: vanilla server jar (not Fabric) with `eula=true`,
  `online-mode=false`, `server-ip=127.0.0.1`, random free port, `pause-when-empty-seconds=-1`,
  `nogui`, in `cache/singleplayer/<world>/`; waits for the `Done` log line (timeout 120 s);
  `stop()` sends `stop` on stdin and waits. Capture logs to a file.
- Vanilla client launcher (needed by WS8's recorder and benchmarks): `launch_vanilla(opts)` builds
  the classpath from the version JSON libraries + client jar, extracts natives, passes the standard
  game arguments (`--username`, `--version`, `--gameDir` = `cache/vanilla-game`, `--assetsDir`,
  `--assetIndex`, `--uuid`, `--accessToken` (offline: any string), `--userType`, `--versionType`),
  optional Fabric: install Fabric Loader via Fabric meta (`https://meta.fabricmc.net/v2`) into the
  same game dir and launch `net.fabricmc.loader.impl.launch.knot.KnotClient` with the loader
  libraries, with a `mods/` dir. Everything under `cache/`, never the user's `.minecraft`.
- Microsoft auth: device-code flow (MSA -> Xbox Live -> XSTS -> Minecraft services -> profile)
  with a configurable `client_id` (none shipped; read from config/env `HASTE_MSA_CLIENT_ID`);
  returns tokens WS1's `Account::Microsoft` consumes (coordinate the struct through the lead).
  Tests: unit tests for the request/response parsing with fixtures; no live calls.
- `haste setup` and `haste launch` subcommands in your client module when the lead opens it.

Do not touch: haste-protocol, haste-world, haste-render, haste-jvm.
