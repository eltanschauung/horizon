#!/usr/bin/env python3
"""Package a private portable game with prepared assets and fresh player data (never publish it)."""
import argparse
import json
from pathlib import Path
import stat
import zipfile


def package(build, resources, content, output, platform):
    suffix = '.exe' if platform == 'windows' else ''
    binary = build / ('horizon-client' + suffix)
    if not binary.is_file():
        raise ValueError(f'Missing binary: {binary}')
    metadata = build / 'native-mods.json'
    json.loads(metadata.read_text())
    json.loads((resources / 'horizon-resources.json').read_text())
    mods = sorted(content.glob('*.hmod'))
    if not mods:
        raise ValueError('No content packages supplied')
    files = [('horizon-client' + suffix, binary), ('native-mods.json', metadata)]
    source = Path(__file__).resolve().parent.parent
    for name in ['LICENSE', 'THIRD_PARTY_NOTICES.md']:
        files.append((name, source / name))
    # A prepared resource tree only; exclude preparation logs, metadata and scratch bytecode.
    for path in sorted(resources.rglob('*')):
        relative = path.relative_to(resources)
        if any(part in {'classes', 'client-classes'} for part in relative.parts):
            continue
        if path.suffix == '.log' or path.name in {'.horizon-prepare.lock', 'horizon-prepare.json'}:
            continue
        if path.is_symlink():
            raise ValueError(f'Resources must be self-contained: {relative}')
        if path.is_file():
            files.append(('resources/' + relative.as_posix(), path))
    files.extend(('data/horizon-mods/' + p.name, p) for p in mods)
    output.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation: never overwrite a previous archive or accidentally package its user data.
    try:
        with zipfile.ZipFile(output, 'x', zipfile.ZIP_DEFLATED, compresslevel=3) as archive:
            for name, path in files:
                archive.write(path, 'horizon_portable/' + name)
            archive.writestr('horizon_portable/.horizon-portable', '1\n')
            archive.writestr('horizon_portable/README.txt',
                'Horizon portable (' + platform + ' x86-64)\n\n'
                'Extract the entire ZIP to a writable folder. Run horizon-client' + suffix + '.\n'
                'No launcher, Java, Rust build or Minecraft download is needed.\n'
                'Linux requires glibc 2.44 or newer, a Vulkan-capable GPU/driver and desktop audio/window libraries.\n'
                'Windows requires a supported graphics driver.\n\n'
                'Choose your character before playing. This copy starts with no saves or account credentials.\n'
                'Player files stay in data/. The first multiplayer connection creates data/portable-identity.key.\n'
                'Keep that file private and back it up: it is your portable player identity.\n'
                'Copying your used data folder keeps the same identity; fresh extractions create different ones.\n'
                'Portable clients need an offline server or a Horizon server with allow_portable enabled.\n'
                'Microsoft-only servers still require Microsoft authentication through the launcher.\n\n'
                'This private package contains your prepared Minecraft assets; do not publish it as a public release.\n')
            if platform == 'linux':
                entry = zipfile.ZipInfo('horizon_portable/Start-Horizon.sh')
                entry.create_system = 3
                entry.external_attr = (stat.S_IFREG | 0o755) << 16
                archive.writestr(entry, '#!/bin/sh\nexec "$(dirname "$(readlink -f "$0")")/horizon-client" "$@"\n')
            else:
                archive.writestr('horizon_portable/Start-Horizon.cmd', '@echo off\r\nstart "" "%~dp0horizon-client.exe" %*\r\n')
        if output.stat().st_size >= 5_000_000_000:
            raise ValueError('Portable archive exceeds the 5 GB artifact limit')
        with zipfile.ZipFile(output) as archive:
            if archive.testzip() is not None:
                raise ValueError('Archive integrity check failed')
            entries = archive.namelist()
            allowed = {'horizon_portable/data/horizon-mods/' + p.name for p in mods}
            assert all(not n.startswith('horizon_portable/data/') or n in allowed for n in entries)
    except Exception:
        # Preserve an existing ZIP on an exclusive-open failure; delete only this attempt's new file.
        if not isinstance(__import__('sys').exc_info()[1], FileExistsError):
            output.unlink(missing_ok=True)
        raise
    print(f'{output}: {output.stat().st_size:,} bytes; fresh data, {len(mods)} content mods')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['build', 'resources', 'content', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--platform', choices=['linux', 'windows'], required=True)
    args = parser.parse_args()
    package(args.build, args.resources, args.content, args.output, args.platform)


if __name__ == '__main__':
    main()
