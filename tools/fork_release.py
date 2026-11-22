#!/usr/bin/env python3
"""Build portable fork archives from the exact SDK and a frozen package set."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import zipfile

from check_glibc import FLOOR_TEXT, check_binaries
from fork_workflow import PACKAGE_DIRS, collect_sdk_packages, digest, unpack, verify_about


def archive(folder, output, windows):
    if windows:
        with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as package:
            for file in sorted(folder.rglob("*")):
                if file.is_file():
                    package.write(file, file.relative_to(folder.parent))
    else:
        with tarfile.open(output, "w:gz") as package:
            package.add(folder, arcname=folder.name)
    if output.stat().st_size >= 5_000_000_000:
        raise ValueError("Release archive exceeds 5 GB")


def active_package_plan(plan):
    manifest = json.loads((plan / "manifest.json").read_text())
    if (manifest.get("schema") != 2
            or not re.fullmatch(r"[a-f0-9]{40}", manifest.get("game_revision", ""))):
        raise ValueError("Expected a frozen SDK workflow manifest")
    packages = collect_sdk_packages(plan / "inputs", manifest["game_revision"])
    def records(entries):
        return {e["id"]: (e["kind"], e["version"], e["sha256"], e.get("source_sha256"))
                for e in entries}
    if (records(entry for _, entry in packages) != records(manifest["packages"])
            or len(packages) != len(manifest["packages"])):
        raise ValueError("The package hashes differ from the frozen manifest")
    return manifest, packages


def stage_packages(scratch, packages):
    packaged = scratch / "fork-mods"
    selection = []
    source = scratch / "native-input"
    for package, entry in packages:
        name = entry["id"]
        kind = entry["kind"]
        target = packaged / PACKAGE_DIRS[kind] / (name + ".hmod")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(package, target)
        selection.append(f"mod.{name}:true\n")
        if kind == "native":
            unpack(package, source / name)
        else:
            # The SDK build reads the bundled content beside its native input.
            content = scratch / PACKAGE_DIRS[kind] / target.name
            content.parent.mkdir(exist_ok=True)
            shutil.copy2(package, content)
    text = "".join(selection)
    (scratch / "horizon-mods.txt").write_text(text)
    (packaged / "default-selections.txt").write_text(text)
    return source, packaged


def build_client(launcher, scratch, source, manifest):
    launch_home = scratch / "launcher"
    env = dict(os.environ,
               HORIZON_LAUNCHER_HOME=str(launch_home),
               HORIZON_HOME=str(scratch / "game"),
               HORIZON_LAUNCHER_NO_SELF_UPDATE="1")
    base = manifest["base"]
    subprocess.run([launcher, "install", base], env=env, check=True)
    info = json.loads(subprocess.check_output(
        [launcher, "fork-info", "--base", base], env=env, text=True))
    if (info["sdk"] != {"tag": base, "revision": manifest["game_revision"]}
            or info["revision"] != manifest["launcher_revision"]
            or info.get("dirty")):
        raise ValueError("The installed SDK or launcher differs from the frozen build")
    command = [launcher, "native-build", source, "--instance", "horizon",
               "--version", base, "--profile", manifest["profile"], "--trust-native-code"]
    subprocess.run(command, env={**env, "HORIZON_FORK_PREFLIGHT": "1"}, check=True)
    subprocess.run(command, env=env, check=True)
    instances = json.loads((launch_home / "instances.json").read_text())
    version = instances["instances"]["horizon"]["version"]
    if not re.fullmatch(r"native-[a-z0-9-]+", version):
        raise ValueError("The SDK did not select a native build")
    folder = launch_home / "fork-versions" / version
    record = json.loads((folder / "native-mods.json").read_text())
    verify_about(manifest, record, "client")
    return folder, record


def distribute(args):
    windows = args.platform == "windows"
    suffix = ".exe" if windows else ""
    platform = "windows-x64" if windows else "linux-x86_64"
    manifest, packages = active_package_plan(args.plan)
    args.scratch.mkdir(parents=True)
    source, packaged = stage_packages(args.scratch, packages)
    built, record = build_client(args.launcher.resolve(), args.scratch.resolve(), source.resolve(), manifest)
    executable = built / ("horizon-client" + suffix)
    folders = ("exporters",)
    programs = ("horizon-client", "horizon-ctl", "horizon-prepare", "horizon-server")
    libs = sorted(file for file in built.iterdir()
                  if re.fullmatch(r"[A-Za-z0-9_+.-]+\.(dll|so(?:\.[0-9]+)*)", file.name))
    runtime = [built / (program + suffix) for program in programs if (built / (program + suffix)).is_file()]
    if executable not in runtime:
        raise ValueError("The SDK's client executable is missing")
    for lib in record.get("libraries", []):
        path = built / lib["file"]
        if path not in libs or digest(path) != lib["sha256"]:
            raise ValueError(f"SDK library missing or changed: {lib['file']}")
    if not windows:
        check_binaries([args.launcher, *runtime, *libs])
    game = args.out / ("horizon-" + platform)
    launcher = args.out / ("horizon-launcher-" + platform)
    game.mkdir(parents=True)
    launcher.mkdir()
    for file in runtime + libs + [built / "native-mods.json"]:
        shutil.copy2(file, game / file.name)
    for name in ("horizon-wasm-std.wasm", "LICENSE", "THIRD_PARTY_NOTICES.md"):
        if (built / name).is_file():
            shutil.copy2(built / name, game / name)
    for name in folders:
        if (built / name).is_dir():
            shutil.copytree(built / name, game / name)
    shutil.copytree(built / "horizon-native-mods", game / "horizon-native-mods")
    shutil.copytree(packaged, game / "fork-mods")
    if windows:
        for name in ("libopus-0.dll", "OPUS-LICENSE.txt"):
            shutil.copy2(args.runtime / name, game / name)
    shutil.copy2(args.launcher, launcher / ("horizon-launcher" + suffix))
    note = "Run horizon-launcher and sign in to prepare the game. Multiplayer: bantculture.com.\n"
    if not windows:
        note += f"Linux x86-64 requires glibc {FLOOR_TEXT} or newer, Vulkan, ALSA and libudev.\n"
    (game / "README.txt").write_text(note, encoding="utf-8")
    (launcher / "README.txt").write_text(note, encoding="utf-8")
    for folder in (game, launcher):
        archive(folder, args.out / (folder.name + (".zip" if windows else ".tar.gz")), windows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("plan", "scratch", "launcher", "out"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--runtime", type=Path)
    parser.add_argument("--platform", choices=("linux", "windows"), required=True)
    args = parser.parse_args()
    if args.platform == "windows" and not args.runtime:
        parser.error("--runtime must contain the Windows Opus files")
    try:
        distribute(args)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        parser.exit(1, str(error) + "\n")


if __name__ == "__main__":
    main()
