# setup-cli: the `haste` commands a player uses, and online-mode account data

Worker: GLM. Worktree `$HASTE_WT/setup-cli`, branch `setup-cli`, test server port **25618**.
Read `docs/briefs/COMMON.md` first.

You own `crates/haste-setup` and a new file `crates/haste-client/src/launch.rs`. `main.rs` is the
lead's: write in your report the exact lines `main.rs` needs (a `mod launch;` line and the
subcommand dispatch), and keep `launch.rs` self-contained so that wiring is a few lines.

## Where it stands
`haste-setup` has `ensure_installed` (Mojang downloads, JDK, data generator), `Config` (TOML),
`LocalServer`, a vanilla/Fabric launcher, and the Microsoft device-code login steps (`msa.rs`;
the `client_id` comes from `HASTE_MSA_CLIENT_ID`, none is shipped). The `haste` binary has no
subcommands for any of it.

## Tasks, in order (commit after each)
1. **Subcommands in `launch.rs`** (clap derive, `#[derive(clap::Subcommand)] pub enum Command`
   plus `pub fn run(cmd: Command) -> anyhow::Result<()>`):
   - `haste setup`: `ensure_installed` with a readable progress line; idempotent and fast when
     everything is present (report the time of a no-op run).
   - `haste login`: device-code flow (prints the URL and code), stores tokens in `Config`;
     clear error when `HASTE_MSA_CLIENT_ID` is unset. `haste logout`.
   - `haste singleplayer [--world <name>]`: starts `LocalServer` on a free port with the world
     under the config dir, and returns the address for the client to join (the lead wires the
     join; give a `pub async fn start_singleplayer(world: &str) -> anyhow::Result<(LocalServer, String)>`).
   - `haste vanilla [--fabric]`: launches the real vanilla client through the existing launcher
     (for side-by-side comparison). Do not run it in tests: it opens a window.
   - Defaults from `Config` (username, render distance, last server) for the play command; the
     lead merges them with the existing flags in `main.rs`.
2. **Online-mode account data.** For chat signing on online-mode servers the client needs the
   player's profile key pair from `https://api.minecraftservices.com/player/certificates`
   (POST with the Minecraft access token; response: PEM private and public key, the
   Mojang-signed public key signature `publicKeySignatureV2`, expiry and refresh times; see
   minecraft.wiki "Mojang API" / "Microsoft authentication"). Add
   `pub async fn player_certificates(client: &reqwest::Client, mc_access_token: &str) -> anyhow::Result<PlayerCertificates>`
   to `msa.rs` with the parsed keys (raw DER bytes, no crypto dependency needed beyond
   `base64`), cache them in `Config` until their refresh time, and a unit test on a recorded
   response shape you write by hand. Also the token refresh path: an expired Minecraft token is
   refreshed from the stored refresh token without user interaction (`ensure_fresh_account`).
3. **Smoke script** `scripts/smoke.sh`: builds release once, starts `scripts/test-server.sh smoke
   <port> 120`, waits for "Done", runs `haste --headless --server 127.0.0.1:<port> --seconds 20`,
   greps the server log for kicks / "moved wrongly" / exceptions, prints PASS/FAIL, stops the
   server. Single-line invocations only, POSIX bash, no sudo.

## Done
`cargo test -p haste-setup` and clippy clean (`-p haste-client` too for `launch.rs`, once the
lead wires it; until then make `launch.rs` compile by adding `mod launch;` locally on your branch
and say so). `scripts/smoke.sh` run output pasted in the report.
