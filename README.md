# NTM Oxidized

A port of HBM's Nuclear Tech Mod to [Horizon](https://github.com/horizon-rust/horizon), the Rust
reimplementation of Minecraft: Java Edition 26.3.

## What works

- Ores with world generation (titanium, lead, tungsten and fourteen more), ingots, powders and smelting.
- Around 850 simple items and 300 blocks (building blocks, slabs, stairs, glass, parts) with their models,
  names, tags, loot and crafting recipes.
- The HE power network, a port of NTM's UNINOS and PowerNetMK2: red copper cable, wood-burning generator,
  network analysis tool.
- The shredder, with blades and over 200 recipes.
- Battery packs and the battery socket: modes, priorities and redstone control.
- The blast furnace (steel and alloys) and the burner press with its stamps.

Machines share one framework in `ntm-core`: their logic, slots and screens are described there once, and
both branches drive them through the same code. New machines are added on it, a few at a time.

## Layout

| Path | What it is |
|---|---|
| `crates/ntm-core` | All game logic: networks, machines, batteries. Plain Rust, no Horizon dependency, unit tested. |
| `crates/ntm-content` | The hand-written content mod: block and item declarations, menus, models, recipes, names. |
| `crates/ntm-direct` | Branch 1: a native mod that hooks Horizon through mixins, pinned to one Horizon revision. |
| `crates/ntm-api` | Branch 2: the same features through Horizon's public mod API only; gaps are marked `API gap`. |
| `generated/` | Content converted from data. Never edited by hand; hand-written files win over it. |
| `textures/` | The textures, as a resource pack. Many are placeholders still being redrawn. |
| `assemble.py` | Builds the folders the Horizon launcher takes into `dist/`. |

The logic lives once in `ntm-core`; both branches are thin adapters around it. When Horizon's API gains a
feature, that piece moves from `ntm-direct` to `ntm-api`.

## Building

```sh
cargo test                 # ntm-core's tests
python assemble.py         # dist/ntm_direct, dist/ntm_api, dist/ntm_content
```

The launcher builds a native mod into the game from its folder, then runs it with the content mod:

```sh
horizon-launcher native-build dist/ntm_direct --instance <instance> --profile dev --trust-native-code
horizon-launcher play --instance <instance> --world <world> --arg=--dev-mod --arg=dist/ntm_content
```

Both bundles name the Horizon revision they are built against in `horizon-mixin.toml` (`horizon_revision`).
