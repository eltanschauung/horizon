"""Source-controlled defaults, used to freeze and install the fork distribution."""
from pathlib import Path, PurePosixPath
import json
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.request
import zipfile
REMOVED={'lambdynamiclights_horizon','lambdynamiclights','sleep_threshold','corpse_native'}
ADDED=('betterdynamiclights','horizon_multiplayer_sleep','horizonmatica','gravestone-core','minimap','yukari_gap_native')
def published_main(checkout):
    remote = subprocess.check_output(
        ['git', '-C', checkout, 'ls-remote', 'origin', 'refs/heads/main'], text=True,
    ).split()
    if len(remote) != 2 or remote[1] != 'refs/heads/main':
        raise ValueError(f'Cannot resolve published main for {checkout.name}')
    return remote[0]


def lunatic_packages(root, destination):
    from fork_workflow import package
    checkout = root.parent / 'Lunatic'
    revision = published_main(checkout)
    if subprocess.run(['git', '-C', checkout, 'cat-file', '-e', revision], capture_output=True).returncode:
        subprocess.run(['git', '-C', checkout, 'fetch', 'origin', 'main'], check=True)
    tools = checkout / 'target/tools'
    destination.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='horizon-lunatic-source-') as directory:
        source = Path(directory)
        with subprocess.Popen(['git', '-C', checkout, 'archive', revision], stdout=subprocess.PIPE) as archive:
            with tarfile.open(fileobj=archive.stdout, mode='r|') as contents:
                contents.extractall(source, filter='data')
            if archive.wait():
                raise ValueError(f'Cannot archive Lunatic {revision}')
        command = [sys.executable, source/'tools/build.py', '--horizon', root, '--tools-target', tools]
        if (tools/'debug/cargo-horizon-mod').is_file():
            command.append('--skip-tool-build')
        subprocess.check_output(command, text=True)
        outputs = source/'target/horizon-mod'
        paths = [destination/'lunatic.hmod', destination/'lunatic.hnmod']
        content, native = (package(outputs/path.name, path.suffix=='.hnmod') for path in paths)
        if content['id'] != 'lunatic' or native['id'] != 'lunatic' or content['version'] != native['version']:
            raise ValueError('Lunatic content and native package identities must agree')
        for path in paths:
            shutil.copy2(outputs/path.name, path)
    return revision, *paths


def native_packages(root):
    native=root.parent/'horizon-mods/native'
    paths=[]
    for name in ADDED:
        subprocess.check_output(['python3',native/'tools/native.py','package',name],text=True)
        # The packager has one deterministic destination; do not parse progress text.
        bundle=next(p for p in (native/name/'mixins').iterdir() if (p/'horizon-mixin.toml').is_file())
        mod=tomllib.loads((bundle/'horizon-mixin.toml').read_text())['mod']
        paths.append(native/'target'/(mod['id']+'-'+mod['version']+'.hnmod'))
    return paths + release_packages(root, "native")

def content_packages(root):
    from fork_workflow import digest
    path = root.parent / 'horizon-mods/native/gravestone-core/gravestone.hmod'
    if digest(path) != '2695c4885fedfe252271229cc33e771720c44b3c9b5f9230cb10b0b20e9da78f':
        raise ValueError('Gravestone assets differ from the pinned registry release')
    ports = [compatible_content(p) for p in release_packages(root, "content")]
    return [path, *ports, emojiful_assets(root)]


def release_packages(root, kind):
    from fork_workflow import digest
    entries = json.loads((root.parent / "horizon-mods/default-release-packages.json").read_text())
    cache = root / ".local" / "distribution-packages"
    paths = []
    for entry in entries:
        if entry["kind"] != kind:
            continue
        url, expected = entry["url"], entry["sha256"]
        folder = cache / expected
        folder.mkdir(parents=True, exist_ok=True)
        path = folder / url.rsplit("/", 1)[-1]
        if not path.is_file() or digest(path) != expected:
            with tempfile.NamedTemporaryFile(dir=folder, delete=False) as download:
                temp = Path(download.name)
                try:
                    with urllib.request.urlopen(url, timeout=60) as response:
                        shutil.copyfileobj(response, download)
                    download.flush()
                    if digest(temp) != expected:
                        raise ValueError(f"Checksum mismatch for {url}")
                    temp.replace(path)
                finally:
                    temp.unlink(missing_ok=True)
        paths.append(path)
    return paths


def package_files(path):
    with zipfile.ZipFile(path) as source:
        files = {}
        for entry in source.infolist():
            name = PurePosixPath(entry.filename)
            if name.is_absolute() or ".." in name.parts or entry.filename in files:
                raise ValueError(f"Unsafe package member: {entry.filename}")
            if not entry.is_dir():
                files[entry.filename] = source.read(entry)
    return files


def write_package(output, files):
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(dir=output.parent, delete=False) as temp:
        path = Path(temp.name)
        try:
            with zipfile.ZipFile(temp, "w") as dest:
                for name, data in sorted(files.items()):
                    dest.writestr(zipfile.ZipInfo(name), data)
            temp.flush()
            path.replace(output)
        finally:
            path.unlink(missing_ok=True)
    return output


def compatible_content(path):
    if path.stem not in {"bibliocraft", "yungsextras"}:
        return path
    files = package_files(path)
    original = files["horizon-mod.toml"].decode()
    # The two data-only releases carry SDK metadata our content manifest rejects.
    # Keep the exact records and assets, and remove only the unused manifest keys.
    header, _, _ = original.partition("\n[native]\n")
    manifest = "\n".join(line for line in header.splitlines()
        if not line.startswith(("license = ", "homepage = ", "issues = ")))
    expected = tomllib.loads(original)
    for key in ("native", "license", "homepage", "issues"):
        expected.pop(key, None)
    if tomllib.loads(manifest) != expected or "mod.wasm" in files:
        raise ValueError(f"Unexpected content manifest changes: {path.name}")
    files["horizon-mod.toml"] = (manifest + "\n").encode()
    return write_package(path.parent / "compatible" / path.name, files)


def emojiful_assets(root):
    pack, = release_packages(root, "font-pack")
    output = pack.with_name("emojiful_twemoji.hmod")
    manifest = '''id = "emojiful_twemoji"
name = "Emojiful Twemoji"
version = "0.1.1"
description = "The Twemoji font and textures from the Emojiful 0.1.1 resource pack."
authors = ["lavoide", "HorizonStudio", "Twemoji contributors"]
api = "0.2"
side = "client"
namespaces = ["emojiful"]
capabilities = ["registries"]
'''
    # Use the content update channel so glyphs arrive together with the native mod.
    files = package_files(pack)
    files["horizon-mod.toml"] = manifest.encode()
    files["assets/emojiful_twemoji/font/horizon_default.json"] = files.pop(
        "assets/minecraft/font/default.json"
    )
    files.pop("pack.mcmeta")
    for notice in ("LICENSE-GRAPHICS.txt", "CREDITS.txt"):
        files[f"assets/emojiful_twemoji/licenses/{notice.lower()}"] = files.pop(notice)
    return write_package(output, files)


def patch_content(root,source,destination,mod_id):
    patch=root.parent/'horizon-mods/content-fixes'/mod_id
    if not patch.is_dir():return
    with zipfile.ZipFile(source) as z:files={n:z.read(n) for n in z.namelist() if not n.endswith('/')}
    for p in sorted(patch.rglob('*')):
        if p.is_file():files[p.relative_to(patch).as_posix()]=p.read_bytes()
    with zipfile.ZipFile(destination,'w',zipfile.ZIP_DEFLATED) as z:
        for n,b in sorted(files.items()):
            info=zipfile.ZipInfo(n,(2020,1,1,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;z.writestr(info,b)

def install_defaults(root):
    import shutil
    from fork_workflow import homes, package
    _,game=homes();folder=game/'horizon-native-mods';folder.mkdir(parents=True,exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='horizon-lunatic-bundles-') as directory:
        _, lunatic_content, lunatic_native = lunatic_packages(root, Path(directory))
        content_home = game/'horizon-mods'
        content_home.mkdir(parents=True, exist_ok=True)
        shutil.copy2(lunatic_content, content_home/lunatic_content.name)
        shutil.copy2(lunatic_native, folder/lunatic_native.name)
    native = native_packages(root)
    content = content_packages(root)
    content_folder = game/'horizon-mods'
    content_folder.mkdir(parents=True, exist_ok=True)
    for src in content:
        info = package(src, False)
        destination = content_folder / (info['id'] + '.hmod') if info['id'] == 'yukari_gap' else content_folder / src.name
        if info['id'] == 'yukari_gap':
            temp = destination.with_suffix('.hmod.new')
            patch_content(root, src, temp, info['id'])
            temp.replace(destination)
        else:
            shutil.copy2(src, destination)
    for src in native:
        info=package(src,True);shutil.copy2(src,folder/(info['id']+'.hnmod'))
    for p in list(folder.glob('*.hnmod')):
        if package(p,True)['id'] in REMOVED:
            import hashlib
            stamp=hashlib.sha256(p.read_bytes()).hexdigest()[:16]
            backup=game/'horizon-mods/.horizon-mod-data/replaced-native-defaults'/(p.stem+'-'+stamp+'.hnmod')
            backup.parent.mkdir(parents=True,exist_ok=True)
            if not backup.exists():shutil.copy2(p,backup)
            p.unlink()
    bop=game/'horizon-mods/biomesoplenty.hmod'
    if bop.is_file():
        temp=bop.with_suffix('.hmod.new');patch_content(root,bop,temp,'biomesoplenty');temp.replace(bop)
