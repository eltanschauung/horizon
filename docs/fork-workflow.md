# Frozen build workflow

Run from the Horizon checkout. Existing `build`, `install`, Lunatic-only and direct-play
commands still work. New commands print one JSON summary; compiler output stays in
logs beside the manifest. Requires the Linux x86-64 fork launcher installed with
`HORIZON_LAUNCHER_VERSION=0.2.58 ./tools/fork.py install launcher` after committing source.

```sh
./tools/fork.py manifest --output .local/my-build/manifest.json
./tools/fork.py preflight .local/my-build/manifest.json --target both
./tools/fork.py build-plan .local/my-build/manifest.json --target both
./tools/fork.py deploy .local/my-build/manifest.json --target both
./tools/fork.py deploy .local/my-build/manifest.json --target both --apply
./tools/fork.py prune-builds --dry-run
./tools/fork.py prune-builds --apply
```

## Manifest and builds

`manifest` snapshots installed native/content packages and the exact launcher binary
into a fresh private directory under `.local/build-plans/`. It records full game,
launcher and horizon-mods revisions, compiler version, profile, package versions
and SHA-256 hashes. Default game revision is local committed HEAD; publish it before
building. The frozen launcher reports its compiled source provenance; unknown/dirty
launcher builds cannot create a plan. This is build provenance, not an account export.

Optional: `--revision SHA`, `--base INSTALLED_VERSION`, `--instance NAME`,
`--profile performance|release`, `--mods id,id` to restrict the native set.
All installed content packages are captured. Inspect the JSON before building;
this snapshots installed packages, not unbuilt sibling-repository edits or runtime
preferences. Do not commit the private snapshot/binaries.

`preflight` stages and checks the actual frozen mod set using the launcher's managed
compiler, Cargo home, target directory, features and profile. It creates no installed
build, exports no engine, and leaves selection unchanged. It may prepare/download
missing build dependencies once. Client/server checks run sequentially through the
shared native stage. This replaces a separate unmodded/debug-cache compatibility check.

`build-plan --target client|server|both` builds the same pinned commit for each target,
ignoring cached mutable `main` lookups. Missing mods, wrong profiles and changed
snapshots fail. Client builds select the instance through the existing launcher;
server builds export an engine and build the matching supervisor with the same
managed compiler/cache. Artifact hashes are recorded in `artifacts.json`. A repeated
server export keeps the previous one as `server-export.previous`; inspect/remove that
old export before replacing it again. GUI and server feature sets differ, but their
native-mod fingerprints must match for deployment.

## Deployment

`deploy --target local|server|both` previews verified artifacts by default. `--apply`
installs matching packages and selects the local build, and/or stages the VPS update.
Server defaults are `telemazer@74.91.116.171`, `/home/telemazer/lunatic-server`,
`lunatic-horizon.service`; override with `--host`, `--server-dir`, `--service`.

The server step verifies uploaded hashes and runtime libraries, checks player count
again immediately before stopping, backs up replaced files, installs the complete
package set, restarts, and waits for running status, zero crashes and at least 18 TPS.
It skips an occupied server; unknown player count fails. A failed restart rolls back
binaries/packages and restarts the previous service. World/config/account files are
not deployment inputs. The JSON report records package agreement, selection, health,
backup and log locations. Preview does not restart or upload server artifacts.

## Pruning

`prune-builds` previews old managed installations; `--apply` removes candidates only.
All instance references, global selection, detected running executables/mappings and
installations containing player data are retained. Cargo targets, dependency caches,
source, SDKs and worlds are never swept. To clean old exports explicitly, add
`--exports /absolute/export/folder` (repeatable): it must contain `native-mods.json`,
and only named Horizon executables are removed. Inspect the preview first. This does
not prune VPS backups, GitHub releases or arbitrary folders.

## Default Lunatic mod set

Keep these native mods installed/enabled when freezing a full deployment: arcadiandream,
chatheads, classic_music, eatinganimation, justenoughitems, lunatic, minimap, no_copper,
tectonic_horizon and zoom. JEI 1.1.0 and Classic Music 0.1.0 are client features;
include their bundles in both deployment snapshots so clients keep the same defaults.
Content packages include Arcadian Dream, Biomes O’ Plenty and Lunatic.
