# Third-party code

Code reused from other projects, with licence. Keep headers intact. GPL-3.0-compatible only.

| Where in Haste | From | Licence | Notes |
|---|---|---|---|
| `phase2/`, `crates/haste-jvm/java` | https://github.com/bednarjosef/haste-spike | GPL-3.0-or-later (same author) | spike code |
| `crates/haste-assets/src/assets.rs`, `crates/haste-assets/src/assets/` (`resources.rs`, `textures.rs`, `models.rs`, `language.rs`) | Pomme `pomme-client/src/assets.rs` + parts of `world/block/model.rs` | GPL-3.0 | asset index over jar + objects dir, texture variable resolution, font loading (reworked for haste-core `Identifier`, reads the jar through `zip`) |
| `crates/haste-assets/src/model.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | model JSON parsing, face positions/UVs (vanilla `FaceInfo`), element/variant rotations, quad winding |
| `crates/haste-assets/src/multipart.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | blockstate variants/multipart parsing; `when` OR/AND + value alternation written for Haste |
| `crates/haste-assets/src/atlas.rs` | Pomme `pomme-client/src/renderer/chunk/atlas.rs` | GPL-3.0 | shelf packer, sRGB-correct box-filter mip chain, sprite transparency classes, animation first-frame extraction (CPU-side buffers instead of Vulkan) |
| `crates/haste-assets/src/bake.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | `bake_resolved_model` geometry pipeline (per BlockStateId, atlas UV assignment and layer heuristic are new) |
| `crates/haste-render/src/vertex.rs` | Pomme `pomme-client/src/renderer/chunk/mesher.rs` + `packing_consts.rs` | GPL-3.0 | 16-byte packed terrain vertex, position quantization and sprite-local UV packing |
| `crates/haste-render/src/mesher.rs`, `crates/haste-render/src/mesher/` (`sink.rs`, `lighting.rs`, `snapshot.rs`, `merge.rs`, `dispatch.rs`) | Pomme `pomme-client/src/renderer/chunk/mesher.rs` | GPL-3.0 | section meshing (MeshSink, AO/smooth light, cullface culling), snapshot capture, worker-pool design on rayon |
| `crates/haste-render/src/occlusion.rs` | Pomme `pomme-client/src/renderer/chunk/occlusion_graph.rs` | GPL-3.0 | section visibility flood fill + cave-culling walk (planned) |
| `crates/haste-render/src/gui.rs` | Pomme `pomme-client/src/world/block/model.rs` | GPL-3.0 | GUI item lighting: the two GUI light directions for side-lit and front-lit items, two-light Lambert shade, GUI normal transform |
| `crates/haste-protocol/src/nbt/` | Pumpkin `crates/pumpkin-nbt` (https://github.com/Pumpkin-MC/Pumpkin, commit b0c48b2) | GPL-3.0 | NBT read/write; module instead of crate, gzip/codec-ops parts dropped, strict `TAG_End` |
| `crates/haste-protocol/src/transport/` (`mod.rs`, `io.rs`, `encryption.rs`, `framing.rs`, `compression.rs`) | Pumpkin `pumpkin-protocol/src/java/packet_decoder.rs`, `packet_encoder.rs`, stream ciphers from `src/lib.rs` | GPL-3.0 | framing, zlib, AES-128-CFB8; single version, raw id + bytes, CFB8 writer rewritten |
| `crates/haste-protocol/src/wire/varint.rs` | Pumpkin `pumpkin-protocol/src/codec/var_int.rs`, `var_long.rs`, `var_uint.rs`, `var_ulong.rs` | GPL-3.0 | Java LEB128 forms only |
