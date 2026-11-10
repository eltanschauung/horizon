# Horizon fork

- Rust game for Minecraft 26.3: `eltanschauung/horizon`; push to `origin`, never `upstream`.
- Upstream sync preservation checklist for both forks: `FORK_FEATURES.md`.
- Checkout: `/home/xxlib/Desktop/horizon`; from VPS: `ssh horizon-desktop`. Inspect status/diffs; preserve unrelated edits.
- Paths/checks: `docs/architecture.md`, `CONTRIBUTING.md` (read matching sections). Game: `crates/horizon-client/src/game.rs` + `game/`; worldgen: `crates/horizon-world/src/worldgen/`; launcher: `launcher/src/`; mods: sibling repos.

## Build and deploy

Rust/cache config: `.local/fork-build.json`. Run here:

```sh
./tools/fork.py build             # Game + launcher; optimized fast profile
./tools/fork.py install           # Build + deploy both; selects plain fork-local
./tools/fork.py build game        # Or: launcher
./tools/fork.py build modded      # Push committed main; build installed/enabled native mods + game; select in launcher
./tools/fork.py build lunatic     # Package Lunatic + install .hmod; restart game
./tools/fork.py install launcher  # Or: game
./tools/fork.py build --release   # Optional maximum optimization
```

Local commands build edits; deploy affected components only. Native mods: package/install in their repo, then launcher Build with selected mods. It fetches pushed fork `main`; push game changes first; preserve ancestry checks/separate installs. Local installs select the plain fork without native mods. Server builds only for server/multiplayer work; telemazer:25565.

Releases: build and package locally using existing caches, then upload the artifacts; do not use GitHub Actions to build releases.

Frozen builds: `manifest`, `preflight`, `build-plan`, `deploy`, `prune-builds`; see `docs/fork-workflow.md`. Deploy/prune preview by default; `--apply` writes.

## Workflow

- Use `rg`; batch independent reads, retrieve bounded snippets once. Validate dependencies/resource/package formats before compilation. Reuse toolchain, target directory and profile; build after edits.
- Build checks: wait 30–55s, keep session handles, read new output/errors only. Avoid repeated log dumps and unrelated commands during waits.
- Skip tests for small tasks and builds for documentation. Otherwise run focused checks once; broaden only for new failures/unresolved risk.
- Handoff in existing docs or ignored `.local/`: completed/next work, exact commands, validation, running handles. Resume there instead of rereading chat history.

## Safety and feedback

- Before deploying the dedicated server, check its player count; if above 0, ask permission before deployment.
- Preserve accounts, keyring, Minecraft resources, skins and worlds; never restart running apps. Test only with temporary game/launcher/XDG homes.
- Preserve Cargo caches/fingerprints/dependencies. Packages below 5 GB; at the limit retain newest/running builds; exclude caches from cleanup.
- Never commit credentials, Minecraft JARs/assets, prepared resources, logs or player data. Before committing: `git diff --check`, `python3 tools/check_public_tree.py`; add `Co-Authored-By:`. Upstream issues/PRs require a request.
- Finish with validation/limits and commit HEADs for repos committed this turn.
- `/feedback <message>`: receiving server's `<HORIZON_HOME>/feedback.sqlite3` (desktop `~/.local/share/horizon/feedback.sqlite3`), table `feedback(time,message)`; UTC ISO timestamps.
- When desktop and server mods differ, sync both to the version furthest ahead of their shared upstream history, preserving local work.
