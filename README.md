# Create — native Rust port (j2h)

Horizon 26.3 native mod compiled from the Java mod **Create 6.0.10** (NeoForge 1.21.1, the latest
release) by the [j2h](https://github.com/horizon-rust/horizon) Java→Rust transpiler pipeline.

## What is inside

- 34 `[[injection]]` entries on Horizon's **official native hook points** (`native-hooks.json`,
  hooks API 45): block break / right-click / use-item, command registration, server lifecycle —
  mapped from Create's NeoForge `@SubscribeEvent` handlers and Fabric-style callbacks.
- Translated handler bodies (Create's real event logic) in `mixins/create/mod.rs`, running on the
  `JvmVal` bridge: `setBlockState`/`give`/`playSound` reach Horizon's real APIs by name.
- Verified: the bundle applies with `horizon-mixins apply` and **compiles inside
  `horizon-client`** (`cargo check` and full release build, 0 errors).

## What works / what does not (honest)

- Content (blocks/items/recipes/textures) ships separately as a content pack.
- Event-driven logic works through the bridge; kinetic machines (contraptions) need the world-read
  bridge extensions — tracked as the next j2h milestone, not hidden here.

## Install

    launcher mods add-github https://github.com/samton23/create-native-rust --trust-native-code
    launcher native-build

Pinned: Horizon `cd627662525d07920c3ce828b72739ec1a86c4ad`, horizon-mixins `a2af5c0dd4117d51ecd70066ba7a00b4a2c10dab`.
