# Fork features and upstream sync guide

This is the preservation checklist for `eltanschauung/horizon` (game **and launcher**)
and `eltanschauung/horizon-mods`. Features below are intentional fork behavior;
saved player preferences must survive changes to defaults. Update this list when
features move upstream or their implementation moves between modules.

## Horizon: game and launcher

| Preserve | Main places to inspect |
| --- | --- |
| Character Select: catalog names, colors and skins across chat, ChatHeads, Tab, overhead names and system/advancement/death messages; account names are the fallback. Permanent duplicate-name reservations per player/character; announce joins only after identity approval. Catalog defaults: slim, character chat enabled, texture derived from ID. Race, size and starting-item metadata (including per-stack enchantments) support native mods. Content enchantments join the synchronized registry so multiplayer inventories retain them. | `characters.rs`, `characters/`, `title/characters.rs`, `game/character_sync.rs`, `game/names.rs`, `lan/hub.rs`, render chat/Tab code |
| Account skins decoded/cached during startup and installed before world rendering; Steve is always the fallback. | render `skins.rs`, `tab_list.rs`; `game/loading_window.rs`, startup in `game.rs` |
| Fork first-run graphics/options/mod defaults, windowed mode, bundled PACP 2.5.4 above Programmer Art, hidden base pack, default multiplayer entry `bantculture.com`. Existing choices remain authoritative. | render `defaults/`, `settings.rs`, `options_file.rs`, `packs/`; `multiplayer.rs`; launcher bundled-mod setup |
| `/worldspawn` (OP, saved Overworld spawn and facing), saved facing for `/setworldspawn`, `/feedback` → SQLite `feedback(time,message)`, and public `!` commands with normal completion/signing. | `docs/fork-commands.md`, `game/commands.rs`, `horizon-command`, `lan/commands.rs`, `game/remote.rs` |
| BOP/custom-biome content composed with host terrain and Tectonic: climate placement/rarity weights, tint sources, feature environments, per-world generation switches and dimension-specific void/flat generation. Content-mod settings pages and safe resource/registry reloads must remain available. | `worldgen_mods.rs`, `mod_content.rs`, `vanillagen.rs`, `horizon-world/src/worldgen/overrides.rs` and `climate/index.rs`; render `options_ui.rs`, `mod_list.rs` |
| Mod projectile pose/fullbright and scoped hurt-cooldown hooks; positional player death sounds; stable HUD corner allocation before status effects; halved attack recovery and missed-swing lockouts. | `game/native_hooks.rs`, `native-hooks.json`, `health.rs`, `mobs.rs`, `attack_strength.rs`, `game/creatures.rs`, `game/extras.rs` |
| No automatic word filtering, escalating slur mutes/bans or Discord Rich Presence. Operator-issued moderation remains supported. | `horizon-chat-filter`, `game/chat_filter.rs`, client Cargo features/modules, server config |
| Optional installation-key portable identities alongside online accounts, explicitly enabled by the server. | `portable.rs`, `entry.rs`, `lan/conn.rs`, server config and API/reference fixtures |
| Launcher builds **our fork main**, enforces fork ancestry, recognizes our bundled native prebuilds and manages bundled content/native packages together. A build must include **every selected mod**: report errors, never silently select a reduced mod set or fall back to upstream game code. | `launcher/src/fork.rs`, `backend.rs`, `native_mods.rs`, bundled-mod/prebuild metadata |
| Local build/install, Lunatic content copy, selected-mod build and direct-play workflow; locally built releases. | `tools/fork.py`, `tools/fork_workflow.py`, `tools/play_selected.py`, `docs/fork-workflow.md`, `AGENTS.md` |

Paths without a crate prefix above are under `crates/horizon-client/src/`;
“render” means `crates/horizon-client-render/src/`.

## Horizon-mods

| Feature | Source / merge distinction |
| --- | --- |
| Minimap defaults: **small, square, Stone, coordinates on**. Coordinates are floored whole blocks, compact white shadowed text below the map. Black/Stone/Bamboo outlines and coordinate drawing now have upstream counterparts: retain current upstream implementation while preserving our defaults and HUD layout behavior. | `native/minimap/`, especially `mixins/minimap/runtime.rs` and its tests |
| Configurable sleeping threshold; fork default **>=33%**, plus One Player, percentage presets and Vanilla. | `native/sleep-threshold/` |
| Native LambDynamicLights port: configurable client-side item/entity/particle/beam lights, retaining other mods' colored lights. | `native/lambdynamiclights-horizon/` |
| Classic Music: classic background playlists by context, normal timing/volume, reversible toggle; discs unchanged. | `native/classic-music/` |
| No Copper: prevent new copper ore generation; separate offline copper-to-coal world converter. | `native/no-copper/` |

Preserve corresponding registry entries under `mods/`, package hashes/licenses and
compatibility manifests. A feature being in this repository does **not** mean it
is installed or enabled on the desktop/server.

## Sync procedure

1. Record separate game, launcher and mods revisions. Latest completed baseline:
   game `fdc87aecdbd116bf1e08adcd062fcbaea8c2e2ca`, launcher `launcher@0.2.58`
   (`9d524a11f0f67ff8afada00daddbec9e94917815`), mods
   `3eedaed7c1359f3ed9152d91dce982f74d93a8c1`. The launcher lives inside the game
   repo but may need a different revision and matching shared support utilities.
2. Merge rather than replace fork trees. Inspect changed callers, moved modules,
   signatures, dependency versions, generated hook catalogs and tests. Resolve
   each feature above against upstream's current architecture; neither “ours”
   nor “theirs” is a blanket conflict policy. Compare actual diffs to the recorded
   upstream baseline rather than treating every historical fork commit as unique.
3. Lunatic, Arcadian Dream, Tectonic-Horizon and BOP live in **separate sibling
   repositories**. Preserve their work and adapt native sources/generators when
   APIs change. Verify desktop/server package contents, not version labels alone;
   use the furthest-ahead compatible work when installations differ.
4. Follow `AGENTS.md` for checks, builds and deployment. Test affected fork paths,
   build the actual selected native set, and assert `native-mods.json` contains
   all expected IDs with empty `left_out`. Check content packages/settings pages
   too; successful vanilla compilation alone is insufficient.
5. Push source before launcher builds. Mutable GitHub `main` lookups can be cached:
   verify the built revision, package hashes and final selection. Deploy matching
   desktop/server packages, preserve worlds/accounts/settings/caches, and check
   server health. Cleanup old builds only after replacements are verified.

## BOP region footprint

Overworld BOP declarations opt into nested, smoothly warped region cells with
`region_min_quarts: 192`. Conditional seeded draws preserve the configured weight
probabilities while separating footprint from rarity. With four BOP tiers, primary
cells span 3072 blocks, secondary 2048, and rare/extra rare target 1536 blocks.
The bounded shared warp softens boundaries; overlapping tier gates and climate, rivers and
mountains can still clip individual biomes. Nether and declarations without this
option retain the existing Java-oracle-compatible region sampler. Changes affect
newly generated terrain; saved chunks are preserved.
