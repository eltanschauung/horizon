# Horizon fork

- Checkout `/home/xxlib/Desktop/horizon-lunatic/horizon`; game + launcher: `eltanschauung/horizon`, mods: sibling `horizon-mods`. Push to `origin`, never `upstream`; VPS connects via `ssh horizon-desktop`.
- Before upstream sync, read `FORK_FEATURES.md`. Consult only relevant sections of `docs/architecture.md` / `CONTRIBUTING.md`; game `crates/horizon-client/src/game/`, terrain `crates/horizon-world/src/worldgen/`, launcher `launcher/src/`.

## Build and deploy

- Commands: `./tools/fork.py`; cache/toolchain config `.local/fork-build.json` (`CARGO_HOME=~/.cargo`, `RUSTUP_HOME=~/.rustup`); frozen workflow details in `docs/fork-workflow.md` (read when needed).
- Native/upstream work: package/install changed mods, then `manifest`; inspect its package set. Use that same manifest for `preflight`, `build-plan`, and `deploy`; pin exact revisions rather than mutable `main`. Recreate only when inputs change.
- Preflight the affected target(s) before expensive builds; it uses the managed native compiler/cache without installing or selecting a build. Client/server stages run sequentially.
- `build-plan PLAN --target client|server|both` records verified artifacts; client builds select the instance. `deploy PLAN --target local|server|both` previews; `--apply` installs. Inspect JSON for missing mods, hash mismatches, skips and server health; never accept a reduced mod set.
- `prune-builds --dry-run` previews; `--apply` removes eligible old installations. Explicit `--exports DIR` cleans export executables only. Never sweep Cargo targets/dependencies/caches, SDKs, source or player data.
- Small edits: `build game`, `build launcher`, `build lunatic` (also installs content). `install game` selects the plain fork, not native mods. Existing `build modded` pushes committed main and builds installed/enabled native mods.
- Releases build/package locally using existing caches, then upload. Launcher version stamping: `HORIZON_LAUNCHER_VERSION=0.2.58 ./tools/fork.py install launcher` (update the version on the next launcher sync).
- Server builds/deployments only for server/multiplayer work; `telemazer@74.91.116.171`, `~/lunatic-server`, `lunatic-horizon.service`, port 25565. Skip deployment when players are online.

## Efficient work

- Inspect status/diffs; preserve unrelated edits. Use `rg`, batch independent bounded reads, and avoid repeated history/file dumps.
- Prefer workflow JSON summaries; inspect only new errors in the referenced log. During builds, retain session handles and wait 30–55s between checks.
- Reuse the same compiler, target directory and profile. Run focused checks once; broaden only for new failures or unresolved risk. Skip tests for small tasks and all builds for documentation.
- Resume from an existing `.local/` handoff before rereading chat: completed/next steps, exact commands, manifest/log paths and running handles. Keep durable behavior in `FORK_FEATURES.md`, not growing handoffs in this file.

## Safety and feedback

- Preserve accounts, keyring, assets, skins, worlds and settings. Test with temporary game/launcher/XDG homes. Packages below 5 GB; retain newest/running builds and exclude caches from cleanup.
- Desktop/server mods should use the furthest-ahead compatible work in shared upstream history; compare package contents/hashes, not version labels alone.
- Never commit credentials, Minecraft assets, logs or player data. Before committing: `git diff --check`, `python3 tools/check_public_tree.py`; include agent attribution.
- `/feedback`: server's `<HORIZON_HOME>/feedback.sqlite3`, table `feedback(time,message)`, UTC ISO timestamps; desktop `~/.local/share/horizon/feedback.sqlite3`.
- Finish with outcome, validation/limits and commit HEADs for repos committed this turn.
