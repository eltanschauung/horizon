#!/usr/bin/env python3
"""Package a resource-free fork prebuild and its launcher for GitHub releases."""
import argparse
import json
from pathlib import Path
import shutil
import tarfile
import zipfile


def copy(source, destination):
    if source.is_dir():
        shutil.copytree(source, destination, dirs_exist_ok=True)
    else:
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, destination)


def archive(folder, output, windows):
    if windows:
        with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as z:
            for p in sorted(folder.rglob("*")):
                if p.is_file():
                    z.write(p, p.relative_to(folder.parent))
    else:
        with tarfile.open(output, "w:gz") as t:
            t.add(folder, arcname=folder.name)
    if output.stat().st_size >= 5_000_000_000:
        raise SystemExit("Release archive exceeds 5 GB")


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ("source", "binaries", "payload", "launcher", "out"):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--platform", choices=["linux", "windows"], required=True)
    p.add_argument("--tag", required=True)
    a = p.parse_args()
    windows = a.platform == "windows"
    suffix = ".exe" if windows else ""
    platform = "windows-x64" if windows else "linux-x86_64"
    extension = ".zip" if windows else ".tar.gz"
    a.out.mkdir(parents=True, exist_ok=True)
    game = a.out / ("horizon-" + platform)
    launcher = a.out / ("horizon-launcher-" + platform)
    for folder in (game, launcher):
        if folder.exists():
            shutil.rmtree(folder)
        folder.mkdir()
    # Explicit allowlist: never copy prepared assets, accounts, caches or worlds.
    for name in ("horizon-client", "horizon-ctl", "horizon-prepare", "horizon-server"):
        copy(a.binaries / (name + suffix), game / (name + suffix))
    for name in ("LICENSE", "THIRD_PARTY_NOTICES.md", "research-lock.json",
                 "crates/horizon-prepare", "mods/horizon/src/test/java", "tools"):
        copy(a.source / name, game / name)
    for name in ("exporters", "fork-mods", "SOURCES.md"):
        copy(a.payload / name, game / name)
    metadata = json.loads((a.payload / "native-mods.json").read_text())
    metadata["base"] = a.tag
    (game / "native-mods.json").write_text(json.dumps(metadata, indent=2) + "\n")
    copy(a.launcher, launcher / ("horizon-launcher" + suffix))
    copy(a.source / "LICENSE", launcher / "LICENSE")
    instructions = (
        "Horizon Lunatic fork — Windows and Linux x86-64\n\n"
        "Run horizon-launcher, sign into the Microsoft account that owns Minecraft Java, "
        "and press Play. The launcher downloads Minecraft 26.3 from Mojang and prepares it "
        "locally. No Rust build or official launcher is required.\n\n"
        "Lunatic, Arcadian Dream, BOP, Tectonic, minimap, chat heads, eating animation and "
        "zoom are included. Select your character in Character Select. Multiplayer: bantculture.com.\n\n"
        "Your login, skin, settings and worlds stay on your own computer. "
        "Minecraft files and player data are not distributed in this release.\n"
    )
    if not windows:
        instructions += "\nLinux: x86-64, glibc 2.35 or newer, Vulkan-capable graphics, X11/Wayland, ALSA and libudev.\n"
    for folder in (game, launcher):
        (folder / "README.txt").write_text(instructions, encoding="utf-8")
    for folder in (game, launcher):
        output = a.out / (folder.name + extension)
        archive(folder, output, windows)
        print(output)


if __name__ == "__main__":
    main()
