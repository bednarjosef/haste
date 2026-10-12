# Phase 2 material (from haste-spike)

Copied from https://github.com/josefbednar/haste-spike (see its REPORT.md and REPORT-2.md). Not built in
Phase 1; kept so Phase 2 starts from working code.

- `bridge/`: the Mixin handler bridge (ASM): reads `@Mixin`/`@Inject`/`@Shadow`, rewrites handlers into
  static methods whose shadow accesses call Rust natives, generates boolean trampolines. Plus the
  Rust landmark dispatcher and benchmarks (`rust-landmark-src/`).
- `surface/`: the ASM scanner that measured what 30 popular mods touch (2 273 classes, 5 662 methods).
- `oracle-mod/`: the test Fabric mod with the server-side "real Mixin oracle" (`HASTE-VERIFY` line):
  two HEAD injections at different priorities, run on the real 26.3 Fabric server. Keep this: Mixin's
  real behaviour is the oracle, not its docs (HEAD runs lower priority first).

The proxy generator (`haste/proxy/Gen.java`) and runtime (`haste/rt`) moved to `crates/haste-jvm/java`
because Phase 1 uses them for tier 2/3.
