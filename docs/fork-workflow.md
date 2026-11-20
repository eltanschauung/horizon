# Frozen build workflow

Run from the Horizon checkout. Existing `build`, `install`, Lunatic-only and direct-play
commands still work. New commands print one JSON summary; compiler output stays in
logs beside the manifest. Use the committed SDK-compatible fork launcher on Linux x86-64.

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

Optional arguments include `--revision SHA`, `--base INSTALLED_VERSION`, `--instance NAME`
and `--packages-root DIRECTORY`. The package root contains `horizon-native-mods`
and `horizon-mods` directories. Both use SDK `.hmod` archives. Package native code
and content together when they share an ID. A full freeze checks every ID in
`tools/fork-distribution.json` and records the signed base plus the hash of each
native source bundle. Use `--mods id,id` for focused development checks. Deploy from
a full distribution manifest and refresh it when published Lunatic main advances.
Keep private snapshots and binaries outside Git.

`preflight` stages and checks the actual frozen mod set using the launcher's managed
compiler, Cargo home, target directory, features and profile. It creates no installed
build, exports no engine, and leaves selection unchanged. It may prepare/download
missing build dependencies once. Client/server checks run sequentially through the
shared native stage. This replaces a separate unmodded/debug-cache compatibility check.

`build-plan --target client|server|both` builds the same pinned commit for each target,
ignoring cached mutable `main` lookups. Missing mods, wrong profiles and changed
snapshots fail. Client builds select the instance through the existing launcher;
server builds export the SDK engine and carry the supervisor from the signed base. Artifact hashes are recorded in `artifacts.json`. A repeated
server export keeps the previous one as `server-export.previous` during replacement.
Successful verification removes it; a failed replacement preserves it for recovery
and must be inspected before retrying. GUI and server feature sets differ, but their
native-mod fingerprints must match for deployment.

## Deployment

`deploy --target local|server|both` previews verified artifacts by default. `--apply`
installs matching packages and selects the local build, and/or stages the VPS update.
Server defaults are `telemazer@74.91.116.171`, `/home/telemazer/lunatic-server`,
`lunatic-horizon.service`; override with `--host`, `--server-dir`, `--service`.

The server step verifies uploaded hashes and runtime libraries, checks player count
again immediately before stopping, backs up replaced files, installs the complete
package set, restarts, and waits for running status, zero crashes and at least 18 TPS.
It skips an occupied server unless `--restart-occupied` is passed explicitly. Unknown player count fails. A failed restart rolls back
binaries/packages and restarts the previous service. World/config/account files are
not deployment inputs. The JSON report records package agreement, selection, health,
backup and log locations. Preview does not restart or upload server artifacts.

## Portable releases

Put `manifest.json`, `inputs/horizon-native-mods/` and `inputs/horizon-mods/`
from the full frozen manifest into `sdk-packages.zip` and upload it to a draft
release. Run the Fork friends release workflow from the manifest's launcher
revision. Each runner installs the same signed base, runs SDK preflight, builds
the complete package set, and checks the build record before archiving. The final
job uploads both platform pairs after both builds pass.

## Linux releases

Build Linux binaries intended for distribution on Ubuntu 22.04, matching upstream's
glibc 2.35 baseline. Use the Ubuntu 22.04 Linux release job or an equivalent build
environment. Run `python3 tools/check_glibc.py` with the launcher, client, preparation
tools, server and any bundled shared libraries before distributing them. The public
release and private portable packagers enforce this check. A build made locally on a
newer distribution can still be used locally.

## Pruning

`prune-builds` previews old managed installations; `--apply` removes candidates only.
All instance references, global selection, detected running executables/mappings and
installations containing player data are retained. Cargo targets, dependency caches,
source, SDKs and worlds are never swept. To clean old exports explicitly, add
`--exports /absolute/export/folder` (repeatable): it must contain `native-mods.json`,
and only named Horizon executables are removed. Inspect the preview first. This does
not prune VPS backups, GitHub releases or arbitrary folders.

## Default Lunatic mod set

The full native and content package lists are in `tools/fork-distribution.json`.
Native client features such as JEI and Classic Music stay in the manifest so
clients receive the same defaults. Gravestone packages retain the private grave
markers in Minimap. Legacy Corpse inventory files stay in existing world backups
for recovery.

## Local modded shortcut

`./tools/fork.py build modded` first packages and checks the sibling
`horizon-mods/native/gensouworks-horizon` content source, atomically installs its
`.hmod` into the active game home, then starts the normal installed native-mod
build. It reuses Lunatic's cached content packager, building that tool only if
missing. Source/package failures stop the workflow before the native build.
This refreshes content JSON changes such as structure rates without requiring a
manual package-copy step. It does not deploy to the dedicated server.

Successful local builds (`build`, `install`, `build-plan`) and applied local
deployments retain the newest generated resource folder per pipeline in
`<HORIZON_HOME>/.horizon-cache`, one each in `mod-content` and `resourcepacks`.
Small cache files and bundled packs stay. Cleanup defers while a Horizon client
or engine is running. Preflight never purges; deployment previews report candidates.

Successful builds and applied deployments also retain the newest snapshot per
launcher family: `fork-native/server-builds`, `fork-native/source` (downloaded
revision copies), `fork-native/patched`, and native and release installations
under `fork-versions`. Selected, instance-referenced and running builds,
retained workflow bases and the cached build's source revision remain protected.
Compiler activity defers launcher cleanup.

The old `fork-native/target-windows` portable-build scratch directory can also
accumulate several GiB. Cleanup removes recognized outputs from its two
`fast` profiles after three days without writes. Both profiles must be idle
and their Cargo lock files stay in place. Managed compiler targets,
dependencies, SDKs, staging, user source checkouts and player data stay.
Preview deployments report candidates without deleting them.


Workflow cleanup also retires obsolete frozen `inputs/`, `native-input/`,
`server-export/`, `server-export.previous/` and failed `upload/` copies under
validated `.local/` plans in Horizon, Lunatic and horizon-mods. Complete old
exports include duplicate prepared resources, not just executables; leaving
those copies behind previously accumulated about 30 GiB. The newest successful
plan for each target, the active plan and newest pending plan are protected.
A verified replacement retires `server-export.previous`; a failed attempt keeps
it for recovery. Logs, manifests, handoffs, package backups and test worlds stay.

Successful builds and applied deployments run this policy automatically.
`prune-builds --dry-run` previews all cleanup families; `--apply` applies them.
Preflight and deployment previews never delete files. Cleanup shares the workflow
lock and defers during native compilation. Small builds use the same policy
without recompiling any game code.

Abandoned `.native-*.partial` client/server installations are also removed when
no native compiler is active; running or data-bearing partials remain protected.

## Gensokyo Legacy port

The biome port is maintained in `../horizon-mods/native/gensokyo-legacy`.
Package its native code, biome data and assets as one SDK `.hmod` before freezing the
distribution. New biomes affect new chunks only.
