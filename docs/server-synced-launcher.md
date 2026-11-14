# Server-managed launcher

The fork's Rust instances follow the dedicated server automatically. On startup,
every minute while idle, and before Play, the launcher reads the `server-sync`
branch's `manifest.json` from this fork. That channel names the server's exact
Horizon commit and immutable native/content packages, with sizes and SHA-256
checksums. It does not follow `main`, compare display version strings, or run
native bundles obtained from arbitrary multiplayer servers.

The Play button is disabled and grey while updating. It shows **Downloading
Updates...** or **Building...**, with separate elapsed clocks. Downloads that
resume during compilation add to the download clock; they do not reset it.
Version selectors and the Versions page are removed from normal navigation.
Developer CLI version/build commands remain available for maintenance.

Required gameplay/content mods show **Required**, with a disabled enabled switch.
The mutation API rejects disabling or removing them too. Optional presentation
mods retain explicit player choices. Registry/GitHub update paths cannot replace
channel-managed versions. Missing optional choices start enabled.

All downloads are verified before active files change. Native builds use the
existing managed toolchain, staging pipeline and Cargo cache, with an explicit
server revision rather than process-global environment changes. A cached build
is reused only when its commit and enabled source digests match. No selected mod
may be silently left out. Selection happens after package/build verification.
Failed builds restore package files and choices; interrupted transactions recover
from a durable journal. Worlds, screenshots, accounts and unrelated settings are
outside this transaction. Errors are displayed; Play retries the update.

A first installation uses the existing Microsoft/Minecraft ownership and resource
preparation flow before building. Portable installations reuse their prepared
assets. Windows channel inputs include Opus and its license; native builds copy
these sidecars from their base. The first native build still requires internet,
Rust downloads, a working native linker and time to compile. Existing users must
receive this launcher update once; subsequent game/mod updates require no client
release or deletion of installations.

## Server deployment

`tools/fork.py deploy PLAN --target server --apply` now stages immutable update
assets on GitHub before touching the server. After the exact binary/packages pass
the normal server health check, it advances the public channel pointer. A skipped
or failed deployment with different files cannot advance clients. A rollback
keeps the previous pointer. The usual online-player deployment rule is unchanged.

Update asset prereleases (`sync-<input hash>`) hold only native source bundles,
content mods and runtime dependencies. They are automatic transport storage, not
platform game releases for players to install. No Minecraft assets, account files,
worlds or server configuration enter the channel. GitHub authentication is needed
only by the operator publishing a deployment.

To seed the channel from an existing server without changing or restarting it:

```sh
python3 tools/windows_runtime.py --output /tmp/opus-runtime
python3 tools/fork_sync.py --live-root /path/to/server \
  --runtime /tmp/opus-runtime --output /tmp/server-channel --publish
```

Seeding verifies every native bundle's unpacked digest against the compiled
server's `native-mods.json`, then rechecks inputs before publication. Normal
updates use the frozen workflow manifest. Changing deployed files outside this
workflow requires republishing the matching channel; clients deliberately do not
infer a build from mutable repository heads.

Diagnostics use the same channel and policy:

```sh
horizon-launcher server-sync --check --json
horizon-launcher server-sync --json
```

Use temporary launcher/game/XDG homes for tests, never a live player's profile.
