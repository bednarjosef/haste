# haste-setup notes (WS7)

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
- Windows JDK unpack (`unzip`) is unimplemented — WS7 runs on linux; the Adoptium URL is built
  for windows but unpacking a zip is TODO if ever needed.
- `find_existing_minecraft()` checks `versions/26.3/26.3.json` under the standard `.minecraft`
  roots; read-only by construction (only `is_file` checks).
- The ustar reader skips pax/GNU extension header data instead of applying it; the Adoptium
  tarball's plain name/prefix fields are correct, so this is fine for our one tarball.
- log4j config goes to `cache/assets/log_configs/<id>` (matches the vanilla launcher layout).

## Task 2 — not started
