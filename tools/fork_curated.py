"""Source-controlled defaults, applied to frozen inputs only; never to a live installation."""
from pathlib import Path
import importlib.util,json,subprocess,zipfile
REMOVED={'lambdynamiclights_horizon','lambdynamiclights','sleep_threshold'}
ADDED=('betterdynamiclights','horizon_multiplayer_sleep')
def native_packages(root):
    native=root.parent/'horizon-mods/native'
    paths=[]
    for name in ADDED:
        result=subprocess.check_output(['python3',native/'tools/native.py','package',name],text=True)
        # The packager has one deterministic destination; do not parse progress text.
        import tomllib
        bundle=next(p for p in (native/name/'mixins').iterdir() if (p/'horizon-mixin.toml').is_file())
        mod=tomllib.loads((bundle/'horizon-mixin.toml').read_text())['mod']
        paths.append(native/'target'/(mod['id']+'-'+mod['version']+'.hnmod'))
    return paths

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
    for p in list(folder.glob('*.hnmod')):
        if package(p,True)['id'] in REMOVED:
            import hashlib
            stamp=hashlib.sha256(p.read_bytes()).hexdigest()[:16]
            backup=game/'horizon-mods/.horizon-mod-data/replaced-native-defaults'/(p.stem+'-'+stamp+'.hnmod')
            backup.parent.mkdir(parents=True,exist_ok=True)
            if not backup.exists():shutil.copy2(p,backup)
            p.unlink()
    for src in native_packages(root):
        info=package(src,True);shutil.copy2(src,folder/(info['id']+'.hnmod'))
    bop=game/'horizon-mods/biomesoplenty.hmod'
    if bop.is_file():
        temp=bop.with_suffix('.hmod.new');patch_content(root,bop,temp,'biomesoplenty');temp.replace(bop)
