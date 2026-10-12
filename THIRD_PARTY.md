# Third-party code

Code reused from other projects, with licence. Keep headers intact. GPL-3.0-compatible only.

| Where in Haste | From | Licence | Notes |
|---|---|---|---|
| `phase2/`, `crates/haste-jvm/java` | https://github.com/josefbednar/haste-spike | GPL-3.0-or-later (same author) | spike code |
| `crates/haste-assets/src/assets.rs` | Pomme `pomme-client/src/assets.rs` + parts of `world/block/model.rs` | GPL-3.0 | asset index over jar + objects dir, texture variable resolution, font loading (reworked for haste-core `Identifier`, reads the jar through `zip`) |
| `crates/haste-assets/src/model.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | model JSON parsing, face positions/UVs (vanilla `FaceInfo`), element/variant rotations, quad winding |
| `crates/haste-assets/src/multipart.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | blockstate variants/multipart parsing; `when` OR/AND + value alternation written for Haste |
| `crates/haste-assets/src/atlas.rs` | Pomme `pomme-client/src/renderer/chunk/atlas.rs` | GPL-3.0 | shelf packer, sRGB-correct box-filter mip chain, sprite transparency classes, animation first-frame extraction (CPU-side buffers instead of Vulkan) |
| `crates/haste-assets/src/bake.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | `bake_resolved_model` geometry pipeline (per BlockStateId, atlas UV assignment and layer heuristic are new) |
