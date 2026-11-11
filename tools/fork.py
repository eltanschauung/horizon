#!/usr/bin/env python3
"""Build or install this Linux fork, including from a non-interactive SSH session."""

import argparse
import json
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
# Public Horizon OAuth registration, not a credential. Existing logins use this app.
MSA_CLIENT_ID = "331e08f8-172d-43b9-835a-3108c2fcdbf3"
LOCAL_VERSION = "fork-local"


def launcher_home(env):
    data = Path(env.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
    return Path(env.get("HORIZON_LAUNCHER_HOME", str(data / "horizon-launcher")))


def build_environment():
    env = os.environ.copy()
    config_file = ROOT / ".local/fork-build.json"
    config = json.loads(config_file.read_text()) if config_file.exists() else {}
    for key in ("CARGO_HOME", "RUSTUP_HOME", "HORIZON_FORK_RUST_BIN"):
        if key not in env and config.get(key) and Path(config[key]).is_dir():
            env[key] = config[key]
    candidates = []
    if env.get("HORIZON_FORK_RUST_BIN"):
        candidates.append(Path(env["HORIZON_FORK_RUST_BIN"]))
    found = shutil.which("cargo", path=env.get("PATH"))
    if found:
        candidates.append(Path(found).parent)
    candidates.append(Path.home() / ".cargo/bin")
    managed = launcher_home(env) / "rust"
    candidates.extend(sorted(managed.glob("*/bin"), key=lambda p: p.stat().st_mtime, reverse=True))
    for directory in candidates:
        cargo = directory / "cargo"
        if not cargo.is_file():
            continue
        candidate_env = dict(env, PATH=str(directory) + os.pathsep + env.get("PATH", ""))
        try:
            checked = subprocess.run([str(cargo), "--version"], env=candidate_env,
                                     capture_output=True, timeout=15)
        except (OSError, subprocess.TimeoutExpired):
            continue
        if checked.returncode == 0:
            env = candidate_env
            break
    else:
        raise RuntimeError("Rust is missing. Install Rust or set HORIZON_FORK_RUST_BIN to its bin directory.")
    if "CARGO_HOME" not in env and (launcher_home(env) / "cargo").is_dir():
        env["CARGO_HOME"] = str(launcher_home(env) / "cargo")
    env["CARGO_TARGET_DIR"] = str(Path(env.get("CARGO_TARGET_DIR", str(ROOT / "target"))).absolute())
    env.setdefault("HORIZON_MSA_CLIENT_ID", MSA_CLIENT_ID)
    return env, str(cargo)


def build(component, profile, jobs, env, cargo):
    env = dict(env)
    env["HORIZON_FORK_SOURCE_REVISION"] = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    env["HORIZON_FORK_LAUNCHER_TREE"] = subprocess.check_output(["git", "rev-parse", "HEAD:launcher"], cwd=ROOT, text=True).strip()
    env["HORIZON_FORK_SOURCE_DIRTY"] = "1" if subprocess.check_output(["git", "diff", "HEAD", "--", "launcher"], cwd=ROOT) else "0"
    if not env["HORIZON_MSA_CLIENT_ID"] and component in ("all", "launcher"):
        raise RuntimeError("HORIZON_MSA_CLIENT_ID must not be empty: the launcher needs it to renew logins.")
    common = [cargo, "build", "--locked", "--profile", profile, "-j", str(jobs)]
    if component in ("all", "game"):
        subprocess.run(common + ["-p", "horizon-client", "--bin", "horizon-client"],
                       cwd=ROOT, env=env, check=True)
        subprocess.run(common + ["-p", "horizon-client-lib", "--bin", "horizon-ctl"],
                       cwd=ROOT, env=env, check=True)
    if component in ("all", "launcher"):
        subprocess.run(common + ["--manifest-path", str(ROOT / "launcher/Cargo.toml"), "--bin", "horizon-launcher"],
                       cwd=ROOT, env=env, check=True)
    print(f"Built {component} ({profile}) in {env['CARGO_TARGET_DIR']}/{profile}", flush=True)


def read_json(path):
    return json.loads(path.read_text()) if path.exists() else {}


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".fork-", dir=path.parent)
    temporary = Path(temporary)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, indent=2)
            stream.write("\n")
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def atomic_copy(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".fork-", dir=destination.parent)
    os.close(fd)
    temporary = Path(temporary)
    try:
        shutil.copy2(source, temporary)
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


def prepared(path, version):
    try:
        return (read_json(path / "horizon-resources.json").get("minecraft") == version
                and all((path / name).is_file() for name in ("registries.json", "commands.json")))
    except (OSError, ValueError):
        return False


def resource_source(home, settings, version, env):
    if env.get("HORIZON_FORK_RESOURCES"):
        candidates = [Path(env["HORIZON_FORK_RESOURCES"])]
    else:
        selected = home / "fork-versions" / (settings.get("selected") or LOCAL_VERSION)
        candidates = [selected / ".local/horizon-client" / version]
        for namespace in ("fork-versions", "versions"):
            candidates.extend(sorted((home / namespace).glob(f"*/.local/horizon-client/{version}")))
    for candidate in candidates:
        if prepared(candidate, version):
            return candidate
    raise RuntimeError(f"No prepared Minecraft {version} files found. Set HORIZON_FORK_RESOURCES to their folder.")


def install(component, profile, env):
    """Install built artifacts; leave processes, accounts, keyring and game data alone."""
    home = launcher_home(env)
    app = Path(env.get("HORIZON_FORK_INSTALL_DIR", str(Path.home() / ".local/lib/horizon-fork")))
    artifacts = Path(env["CARGO_TARGET_DIR"]) / profile
    names = (["horizon-client", "horizon-ctl"] if component in ("all", "game") else [])
    if component in ("all", "launcher"):
        names.append("horizon-launcher")
    for name in names:
        if not (artifacts / name).is_file():
            raise RuntimeError(f"Missing build artifact: {artifacts / name}")
    # Read metadata and check resources before replacing any installed executable.
    settings = read_json(home / "settings.json")
    instances = read_json(home / "instances.json")
    lock = read_json(ROOT / "research-lock.json")
    version = lock["minecraft"]["version"]
    game = home / "fork-versions" / LOCAL_VERSION
    resources = game / ".local/horizon-client" / version
    source = None
    if component in ("all", "game") and not prepared(resources, version):
        source = resource_source(home, settings, version, env)
        if resources.exists():
            raise RuntimeError(f"Incomplete resources at {resources}; repair them before installing.")
    if component in ("all", "game"):
        if source is not None:
            resources.parent.mkdir(parents=True, exist_ok=True)
            with tempfile.TemporaryDirectory(prefix=".fork-resources-", dir=resources.parent) as folder:
                staged = Path(folder) / "prepared"
                shutil.copytree(source, staged)
                staged.replace(resources)
        for name in ("horizon-client", "horizon-ctl"):
            atomic_copy(artifacts / name, game / name)
        atomic_json(game / "research-lock.json", lock)
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
        dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True))
        atomic_json(game / "fork-source.json", {"repository": "eltanschauung/horizon", "revision": revision,
                                               "dirty": dirty, "profile": profile, "built_locally": True})
        (game / ".installed").touch()
        settings["selected"] = LOCAL_VERSION
        instances.setdefault("instances", {}).setdefault("horizon", {})["version"] = LOCAL_VERSION
        atomic_json(home / "settings.json", settings)
        atomic_json(home / "instances.json", instances)
        print(f"Installed and selected {game}", flush=True)
    if component in ("all", "launcher"):
        atomic_copy(artifacts / "horizon-launcher", app / "horizon-launcher")
        # No GUI or keyring access is needed, including when installing over SSH.
        data = Path(env.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
        desktop = data / "applications/horizon-launcher.desktop"
        executable = str(app / "horizon-launcher")
        quoted = '"' + ''.join('\\' + c if c in '\\"`$' else c for c in executable) + '"'
        desktop.parent.mkdir(parents=True, exist_ok=True)
        desktop.write_text(f"[Desktop Entry]\nType=Application\nName=Horizon Fork Launcher\n"
                           f"Exec={quoted}\nIcon=horizon\nTerminal=false\nCategories=Game;\n"
                           "StartupWMClass=horizon-launcher\n")
        print(f"Installed {app / 'horizon-launcher'} and its application-menu entry", flush=True)
    print("Running apps were left open. Restart them when ready to use the new binaries.", flush=True)


def fork_launcher():
    override = os.environ.get("HORIZON_FORK_LAUNCHER")
    if override:
        return override
    desktop = Path.home() / ".local/share/applications/horizon-launcher.desktop"
    if desktop.exists():
        for line in desktop.read_text().splitlines():
            if line.startswith("Exec="):
                executable = shlex.split(line[5:])[0]
                if Path(executable).is_file():
                    return executable
    found = shutil.which("horizon-launcher")
    if found:
        return found
    raise RuntimeError("Install the fork launcher first or set HORIZON_FORK_LAUNCHER")



def build_worldedit():
    """Keep our default WorldEdit source bundle installed before native selection."""
    source = ROOT.parent / "horizon-mods/native/worldedit/tools/build.py"
    subprocess.run([sys.executable, str(source), "--horizon", str(ROOT), "--install"], check=True)


def build_gensouworks():
    """Refresh the local content port before the launcher reads installed packages."""
    source = ROOT.parent / "horizon-mods/native/gensouworks-horizon"
    if not (source / "horizon-mod.toml").is_file():
        raise RuntimeError(f"Gensouworks source missing: {source}")
    env, cargo = build_environment()
    target = ROOT.parent / "Lunatic/target/tools"
    tool = target / "debug/cargo-horizon-mod"
    if not tool.is_file():
        env["CARGO_TARGET_DIR"] = str(target)
        subprocess.run([cargo, "build", "--locked", "-p", "cargo-horizon-mod", "-j", "2"],
                       cwd=ROOT, env=env, check=True)
    subprocess.run([str(tool), "build"], cwd=source, env=env, check=True)
    package = source / "target/horizon-mod/gensouworks_horizon.hmod"
    subprocess.run([str(tool), "check", str(package), "--json"],
                   cwd=source, env=env, check=True)
    data = Path(env.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
    game_home = Path(env.get("HORIZON_HOME", str(data / "horizon")))
    destination = game_home / "horizon-mods/gensouworks_horizon.hmod"
    atomic_copy(package, destination)
    print(f"Installed Gensouworks content: {destination}", flush=True)


def main():
    if len(sys.argv) > 1 and sys.argv[1] in {"manifest", "build-plan", "preflight", "deploy", "prune-builds"}:
        from fork_workflow import main as workflow_main
        sys.exit(workflow_main(sys.argv[1:]))
    parser = argparse.ArgumentParser(description=__doc__, epilog="Workflow commands: manifest, preflight, build-plan, deploy, prune-builds (see docs/fork-workflow.md).")
    parser.add_argument("action", choices=("build", "install"), help="install builds first, then deploys")
    parser.add_argument("component", nargs="?", default="all", choices=("all", "game", "launcher", "lunatic", "modded"))
    parser.add_argument("--release", action="store_true", help="use slower release builds instead of fast")
    parser.add_argument("--jobs", type=int, default=2, help="parallel compiler jobs (default: 2)")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if sys.platform != "linux":
        parser.error("these local build/install commands currently support Linux")
    if args.component == "modded":
        try:
            # Native builds fetch committed fork main. Never silently omit local source edits.
            changed = subprocess.check_output(["git", "diff", "HEAD", "--name-only"], cwd=ROOT, text=True).splitlines()
            if any(name != "AGENTS.md" for name in changed):
                raise RuntimeError("Commit source changes first; native builds use pushed main")
            branch = subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
            if branch != "main":
                raise RuntimeError("Run build modded from main")
            subprocess.run(["git", "push", "origin", "main"], cwd=ROOT, check=True)
            build_worldedit()
            build_gensouworks()
            subprocess.run([fork_launcher(), "native-build", "--instance", "horizon", "--trust-native-code", "--allow-version-mismatch", "--profile", "release" if args.release else "performance"], cwd=ROOT, check=True)
        except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
            parser.exit(1, f"fork: {error}\n")
        return
    if args.component == "lunatic":
        if args.action != "build":
            parser.error("Lunatic supports build only; install its packages through the launcher")
        if args.release:
            parser.error("--release applies to game/launcher builds, not Lunatic packages")
        lunatic = ROOT.parent / "Lunatic"
        command = [sys.executable, str(lunatic / "tools/build.py"), "--horizon", str(ROOT)]
        if (lunatic / "target/tools/debug/cargo-horizon-mod").is_file():
            command.append("--skip-tool-build")
        try:
            subprocess.run(command, cwd=lunatic, check=True)
            data_home = Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share")))
            game_home = Path(os.environ.get("HORIZON_HOME", str(data_home / "horizon")))
            destination = game_home / "horizon-mods/lunatic.hmod"
            atomic_copy(lunatic / "target/horizon-mod/lunatic.hmod", destination)
            print(f"Installed Lunatic content: {destination}. Restart Horizon to load it.", flush=True)
        except (OSError, subprocess.CalledProcessError) as error:
            parser.exit(1, f"fork: {error}\n")
        return
    try:
        env, cargo = build_environment()
        profile = "release" if args.release else "fast"
        build(args.component, profile, args.jobs, env, cargo)
        if args.action == "install":
            install(args.component, profile, env)
    except (OSError, ValueError, KeyError, RuntimeError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"fork: {error}\n")


if __name__ == "__main__":
    main()
