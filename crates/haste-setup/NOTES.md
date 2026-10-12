# haste-setup notes (setup)

## Task 1: ensure_installed — done

- `mojang.rs`: serde types for `version_manifest_v2.json`, the per-version JSON and the asset
  index. Minimal fields only (downloads, assetIndex, javaVersion, libraries+rules, logging).
  Verified against the real 26.3.json from the spike cache (114 libs, 66 on linux, java 25).
  26.3 has no `natives` classifiers, but the type supports them (`downloads.classifiers` +
  `natives` map) for other versions.
- `download.rs`: single-file download with sha1 verification, skip-if-valid, `<dest>.part` +
  HTTP Range resume (restarts when the server ignores Range; the final sha1 check decides).
  `download_many`: 8-way bounded concurrency, progress events (per-file and per-byte) through an
  unbounded channel. Tests run a local hyper 1.x HTTP server.
- `install.rs`: `ensure_installed` = version JSON (cached at `cache/versions/26.3.json`) ->
  client/server jars -> asset index + objects -> per-OS libraries + log4j config into
  `cache/libraries` (Maven layout from `downloads.artifact.path`) and `cache/assets/log_configs` ->
  JDK (`cache/jdk/bin/java`, else PATH/JAVA_HOME java >= 25, else Temurin 25 via Adoptium API,
  unpacked with a hand-rolled ustar reader with symlink + pax-skip support — the `tar` crate is
  not a workspace dep and adding it just for this seemed heavy; revisit if it bites) ->
  datagen (`-DbundlerMainClass=net.minecraft.data.Main -jar server.jar --reports --all` in
  `cache/datagen/26.3`, log at `cache/datagen/26.3.log`).
- Real-cache run (`cargo test -p haste-setup -- --ignored`): first run downloaded 473 MB assets +
  81 MB libraries (66 jars), jars and reports skipped as expected; second run 3.9 s (sha1 checks
  only). All downloaded sha1s verified.

### Approximations / decisions
- `ensure_installed(progress)` takes `FnMut(Progress) + Send + 'static` (INTERFACES.md says
  `impl FnMut(Progress)`); the 'static bound comes from the concurrency in `download_many`.
- Windows JDK unpack (`unzip`) is unimplemented — setup runs on linux; the Adoptium URL is built
  for windows but unpacking a zip is TODO if ever needed.
- `find_existing_minecraft()` checks `versions/26.3/26.3.json` under the standard `.minecraft`
  roots; read-only by construction (only `is_file` checks).
- The ustar reader skips pax/GNU extension header data instead of applying it; the Adoptium
  tarball's plain name/prefix fields are correct, so this is fine for our one tarball.
- log4j config goes to `cache/assets/log_configs/<id>` (matches the vanilla launcher layout).

## Task 2: config + local server + launcher flow — done

- `config.rs`: `Config` TOML in the platform config dir (`directories::ProjectDirs`), 0600 file /
  0700 dir; `load()` creates defaults on first run. `MsaTokens` shape for protocol (coordinate the final
  struct through the lead — I put `access_token`/`refresh_token`/`expires_at`/`profile{uuid,name}`).
- `server.rs`: `LocalServer::start(world_dir, port)` — vanilla server.jar (not Fabric),
  `eula=true`, `online-mode=false`, `server-ip=127.0.0.1`, `pause-when-empty-seconds=-1`,
  `nogui`, logs to `<world>/server.log`, waits for `Done (` (120 s), `address()` = `127.0.0.1:port`,
  `stop()` sends `stop` on stdin. `free_port()` for a random port. Verified against the real
  server (Done in ~10 s, clean stop).
- `launch.rs`: `launch_vanilla(LaunchOptions)` — classpath from the version JSON libraries +
  client jar, natives extraction (no-op for 26.3), standard game args (`--username --version
  --gameDir=cache/vanilla-game --assetsDir --assetIndex --uuid --accessToken --userType
  --versionType`), JVM native-path args, `net.minecraft.client.main.Main`, logs to
  `<gameDir>/client.log`. `offline_uuid()` = Java's `UUID.nameUUIDFromBytes` (MD5 v3 of
  `OfflinePlayer:<name>`; MD5 implemented inline — RFC vectors tested). Fabric:
  `install_fabric()` fetches the loader profile from meta.fabricmc.net, downloads the 7 loader
  libraries (sha1 from the profile; fabric-loader itself ships without a sha1 — downloaded
  unverified) into `cache/libraries`, launches `KnotClient` with `mods/` under
  `cache/fabric-game`. Both verified live: vanilla boots to Datafixer, Fabric logs
  "Loading Minecraft 26.3 with Fabric Loader 0.19.5".
- `msa.rs`: device-code flow (MSA -> XBL -> XSTS -> MC login -> profile), `client_id` from
  `HASTE_MSA_CLIENT_ID`. Request/response types tested on fixtures; no live calls made (no
  client_id to test with).
- `haste setup` / `haste launch` subcommands: not done — `crates/haste-client` is the lead's
  module, waiting for the go per the brief.

### Bugs found while testing
- `download.rs`: a `.part` file already at full length made the server answer `416` (Range past
  EOF). Now: on 416, verify the part; if valid rename into place, else truncate and retry. Also,
  an empty `sha1` in a spec now means "unverifiable, accept" (Fabric's fabric-loader artifact
  has no checksum in the meta).

## Dependency requests (lead)
- `toml = "1"` — added to the workspace for Config; `toml_edit`/`winnow`/`indexmap` were already
  in the graph via winit, so the addition is small. If you'd rather not have it, the Config code
  can switch to a hand-rolled subset parser.
- `uuid` was already a workspace dep; haste-setup now uses it for offline UUIDs.

