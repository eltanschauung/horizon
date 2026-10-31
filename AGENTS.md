# Horizon fork

- Repository: `eltanschauung/horizon`; push to `origin`, never `upstream`.
- Desktop checkout: `/home/xxlib/Desktop/horizon`. From the VPS, use `ssh horizon-desktop`.
- Preserve our skin changes: Steve fallback, startup caching, textures ready before world rendering.
- Launcher Build uses the fork's pushed `main`; keep its ancestry checks and separate fork installs.

## Commands

Run from the checkout on the desktop, including over SSH:

```sh
./tools/fork.py build                  # Build game + launcher, optimized fast profile
./tools/fork.py install                # Build + install both; select the local game build
./tools/fork.py build game             # Or: launcher
./tools/fork.py install launcher       # Or: game
./tools/fork.py build --release        # Slower build, maximum optimization
```

Rust is found automatically. Optional machine-specific cache/toolchain paths go in ignored
`.local/fork-build.json` (`CARGO_HOME`, `RUSTUP_HOME`, `HORIZON_FORK_RUST_BIN`).
Install preserves accounts, keyring, Minecraft files, skins and worlds; it never restarts running apps.
Use install when asked to deploy changes. The launcher's Build fetches pushed code; these commands
build the local checkout, including uncommitted edits. Native mods are applied by launcher Build;
local game installs select the plain fork.

## Rules

- Never commit tokens, Minecraft JARs/assets, prepared files, logs or player data.
- Test with temporary game/launcher/XDG homes; keep tests away from real worlds and accounts.
- Run relevant tests, `git diff --check` and `python3 tools/check_public_tree.py` before committing.
- Add an agent `Co-Authored-By:` trailer to commits. Don't post upstream issues/PRs unless asked.
- Code map: `docs/architecture.md`. Gameplay conventions and checks: `CONTRIBUTING.md`.
