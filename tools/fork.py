#!/usr/bin/env python3
"""Build or install this Linux fork, including from a non-interactive SSH session."""

import argparse
import json
import os
from pathlib import Path
import shutil
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
    if not env["HORIZON_MSA_CLIENT_ID"] and component in ("all", "launcher"):
        raise RuntimeError("HORIZON_MSA_CLIENT_ID must not be empty: the launcher needs it to renew logins.")
    common = [cargo, "build", "--locked", "--profile", profile, "-j", str(jobs)]
    if component in ("all", "game"):
        subprocess.run(common + ["-p", "horizon-client", "--bin", "horizon-client", "--bin", "horizon-ctl"],
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "install"), help="install builds first, then deploys")
    parser.add_argument("component", nargs="?", default="all", choices=("all", "game", "launcher"))
    parser.add_argument("--release", action="store_true", help="use slower release builds instead of fast")
    parser.add_argument("--jobs", type=int, default=2, help="parallel compiler jobs (default: 2)")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    if sys.platform != "linux":
        parser.error("these local build/install commands currently support Linux")
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
