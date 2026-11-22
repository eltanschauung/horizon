#!/usr/bin/env python3
"""Frozen native build inputs, compact deployment reports and conservative pruning."""
import argparse
import contextlib
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PACKAGE_DIRS = {'native': 'horizon-native-mods', 'content': 'horizon-mods'}
SCHEMA = 2


def digest(path):
    with open(path, 'rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read(path, default=None):
    return json.loads(Path(path).read_text()) if Path(path).exists() else (default or {})


def write(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(path.name + '.new')
    temp.write_text(json.dumps(value, indent=2, sort_keys=True) + '\n')
    temp.replace(path)


def homes():
    data = Path(os.environ.get('XDG_DATA_HOME', Path.home()/'.local/share'))
    roots = read(Path(os.environ.get('XDG_CONFIG_HOME', Path.home()/'.config'))/'horizon-launcher-location/location.json').get('roots', {})
    return (Path(os.environ.get('HORIZON_LAUNCHER_HOME', roots.get('launcher', data/'horizon-launcher'))),
            Path(os.environ.get('HORIZON_HOME', roots.get('game', data/'horizon'))))


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, text=True).strip()


def workspace_root():
    common_dir = Path(git('rev-parse', '--path-format=absolute', '--git-common-dir'))
    return common_dir.parent.parent


def source_commit(checkout, ref):
    return subprocess.check_output(
        ['git', '-C', checkout, 'rev-parse', '--verify', ref + '^{commit}'],
        text=True,
    ).strip()


def safe_id(value):
    if not re.fullmatch(r'[a-z0-9_]{1,64}', value):
        raise ValueError('Invalid package id: '+value)
    return value


def unpack(source, destination):
    with zipfile.ZipFile(source) as z:
        for entry in z.infolist():
            p = Path(entry.filename)
            if p.is_absolute() or '..' in p.parts or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError('Unsafe source bundle member: '+entry.filename)
        if sum(e.file_size for e in z.infolist()) > 5_000_000_000:
            raise ValueError('Source bundle exceeds 5 GB')
        z.extractall(destination)


def collect_sdk_packages(game, revision, wanted=None):
    from fork_sync import identity, native_digest

    required = read(ROOT/'tools/fork-distribution.json')
    seen = set()
    entries = []
    for kind, folder in PACKAGE_DIRS.items():
        for source in sorted((game/folder).glob('*.hmod')):
            if not source.is_file() or source.is_symlink():
                raise ValueError(f'Invalid SDK package: {source}')
            info = identity(source, kind, revision)
            mod_id = safe_id(info['id'])
            if kind == 'native' and wanted is not None and mod_id not in wanted:
                continue
            if mod_id in seen:
                raise ValueError(f'Duplicate package ID: {mod_id}. Bundle native code and its content together.')
            seen.add(mod_id)
            entry = dict(id=mod_id, version=info['version'], sha256=digest(source), kind=kind)
            if kind == 'native':
                entry['source_sha256'] = native_digest(source)
            entries.append((source, entry))
        expected = required[kind] if kind != 'native' or wanted is None else wanted
        found = {entry['id'] for _, entry in entries if entry['kind'] == kind}
        missing = set(expected)-found
        if missing:
            raise ValueError(f'Missing {kind} SDK packages: {", ".join(sorted(missing))}')
    return entries


def snapshot(args):
    from fork import fork_launcher
    h, game = homes()
    revision = git('rev-parse', args.revision or 'HEAD')
    if not re.fullmatch(r'[a-f0-9]{40}', revision):
        raise ValueError('Expected a complete game commit')
    selected = args.base or read(h/'instances.json').get('instances',{}).get(args.instance,{}).get('version') or read(h/'settings.json').get('selected')
    if not selected:
        raise ValueError('No installed base version; supply --base')
    launcher = Path(fork_launcher()).resolve()
    launcher_hash = digest(launcher)
    provenance = json.loads(subprocess.check_output([launcher, 'fork-info', '--base', selected], text=True))
    if provenance.get('dirty') or not provenance.get('revision') or 'sdk-hmod' not in provenance.get('capabilities', []):
        raise ValueError('Install the committed SDK workflow-capable fork launcher first')
    sdk = provenance['sdk']
    if sdk['revision'] != revision:
        raise ValueError(f"Installed base {sdk['tag']} uses {sdk['revision']}; expected SDK {revision}")
    stamp = dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    out = (args.output or ROOT/'.local/build-plans'/stamp/'manifest.json').resolve()
    if out.exists() or (out.parent/'inputs').exists():
        raise ValueError('Choose a fresh manifest directory')
    wanted = {safe_id(part) for part in args.mods.split(',')} if args.mods else None
    packages = collect_sdk_packages((args.packages_root or game).resolve(), revision, wanted)
    workspace = workspace_root()
    lunatic_revision = source_commit(workspace/'Lunatic', args.lunatic_ref)
    mods_revision = source_commit(workspace/'horizon-mods', args.mods_ref)
    toolchain = git('show', provenance['revision']+':tools/ci/rust-version')
    if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+', toolchain):
        raise ValueError('The launcher revision has no exact Rust compiler version')
    entries = []
    for source, entry in packages:
        dest = out.parent/'inputs'/PACKAGE_DIRS[entry['kind']]/(entry['id']+'.hmod')
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, dest)
        if digest(dest) != entry['sha256']:
            raise ValueError(f"Package changed during snapshot: {entry['id']}")
        entries.append(dict(entry, path=str(dest.relative_to(out.parent))))
    frozen_launcher = out.parent/'inputs/launcher'
    shutil.copy2(launcher, frozen_launcher)
    if digest(frozen_launcher) != launcher_hash:
        raise ValueError('Launcher changed during snapshot')
    manifest = {
        'schema': SCHEMA, 'created_utc': stamp, 'game_revision': revision,
        'launcher_revision': provenance['revision'], 'launcher_tree': provenance['launcher_tree'],
        'launcher_sha256': launcher_hash, 'launcher_version': provenance['version'],
        'mods_revision': mods_revision, 'lunatic_revision': lunatic_revision,
        'toolchain': toolchain, 'profile': 'optimized', 'base': sdk['tag'],
        'instance': args.instance, 'packages': entries,
    }
    write(out, manifest)
    return {'manifest':str(out), 'manifest_sha256':digest(out), 'revision':revision,
            'native_mods':sorted(e['id'] for e in entries if e['kind'] == 'native')}


def load(path):
    path = Path(path).resolve()
    m = read(path)
    if m.get('schema')!=SCHEMA or not re.fullmatch(r'[a-f0-9]{40}',m.get('game_revision','')):
        raise ValueError('Invalid workflow manifest')
    for e in m['packages']:
        safe_id(e['id'])
        src = (path.parent/e['path']).resolve()
        if not src.is_relative_to(path.parent) or digest(src)!=e['sha256']:
            raise ValueError('Snapshot changed: '+e['path'])
    if digest(path.parent/'inputs/launcher')!=m['launcher_sha256']:
        raise ValueError('Frozen launcher changed')
    return path,m


def verify_about(manifest, about, target=None):
    expected = {e['id']:(e['version'], e['source_sha256'])
                for e in manifest['packages'] if e['kind']=='native'}
    mods = about['mods']
    actual = {e['id']:(e['version'], e['sha256']) for e in mods}
    if (about['horizon'] != manifest['game_revision'] or about['base'] != manifest['base']
            or expected != actual or len(mods) != len(actual) or about.get('left_out') != []):
        raise ValueError('Build does not match the pinned SDK and complete native mod set')
    if about['rust'] != manifest['toolchain'] or about['profile'] != manifest['profile']:
        raise ValueError('Build compiler/profile differs from the manifest')
    if target and about.get('target') != target:
        raise ValueError('Build is for a different runtime side')
    return sorted(actual)


def runtime_files(folder, target, about):
    binary = 'horizon-client' if target == 'client' else 'horizon-engine'
    files = {binary, 'native-mods.json'}
    if target == 'server':
        files.add('horizon-server')
    if (folder/'horizon-wasm-std.wasm').is_file():
        files.add('horizon-wasm-std.wasm')
    # SDK base/runtime libraries and the compiled mod libraries use $ORIGIN.
    files.update(p.name for p in folder.iterdir()
                 if p.is_file() and re.fullmatch(r'[A-Za-z0-9_+.-]+\.so(?:\.[0-9]+)*', p.name))
    for library in about.get('libraries', []):
        path = library['file']
        if path not in files or digest(folder/path) != library['sha256']:
            raise ValueError(f'SDK mod library missing or changed: {path}')
    for mod in about['mods']:
        prefix = Path('horizon-native-mods')/safe_id(mod['id'])
        source = folder/prefix
        if source.is_symlink() or not (source/'horizon-mod.toml').is_file():
            raise ValueError(f"SDK runtime package is missing: {mod['id']}")
        for path in source.rglob('*'):
            if path.is_symlink():
                raise ValueError(f'SDK runtime package contains a symlink: {path}')
            if path.is_file():
                files.add(path.relative_to(folder).as_posix())
    return {name:digest(folder/name) for name in sorted(files)}


def logged(command, log, env=None, cwd=None):
    with open(log,'w') as stream:
        result = subprocess.run(command, env=env, cwd=cwd, stdout=stream, stderr=subprocess.STDOUT)
    if result.returncode:
        lines = Path(log).read_text(errors='replace').replace('\r','\n').splitlines()
        errors = [line for line in lines if line.startswith(('error','Error','fork:'))]
        raise RuntimeError(json.dumps({'log':str(log),'errors':errors[-8:] or lines[-5:]}))


def build(args):
    p,m = load(args.manifest)
    h,_ = homes()
    # Verify publication, but never substitute a moving main for the frozen hash.
    remote = git('ls-remote','origin','refs/heads/main').split()[0]
    subprocess.run(['git','merge-base','--is-ancestor',m['game_revision'],remote],cwd=ROOT,check=True)
    env = os.environ.copy()
    provenance = json.loads(subprocess.check_output(
        [p.parent/'inputs/launcher', 'fork-info', '--base', m['base']], text=True, env=env))
    if provenance['sdk'] != {'tag':m['base'], 'revision':m['game_revision']}:
        raise ValueError('Installed base no longer matches the frozen SDK')
    if args.command=='preflight':env['HORIZON_FORK_PREFLIGHT']='1'
    else:env.pop('HORIZON_FORK_PREFLIGHT',None)
    source = p.parent/'inputs/native-input'
    if source.exists():shutil.rmtree(source)
    for e in m['packages']:
        if e['kind'] == 'native':
            unpack(p.parent/e['path'], source/e['id'])
    (source.parent/'horizon-mods.txt').write_text(
        ''.join(f"mod.{e['id']}:true\n" for e in m['packages']))
    results = read(p.parent/'artifacts.json')
    # Separate targets are sequential: the launcher owns one reusable staging workspace.
    for target in (['client','server'] if args.target=='both' else [args.target]):
        log = p.parent/(args.command+'-'+target+'.log')
        out = p.parent/('server-check' if args.command=='preflight' else 'server-export')
        command = [str(p.parent/'inputs/launcher'),'native-build',str(source),'--version',m['base'],
                   '--trust-native-code','--profile',m['profile']]
        if target=='client':command+=['--instance',m['instance']]
        else:command+=['--server-output',str(out)]
        if target=='server' and out.exists():
            if args.command=='preflight':raise ValueError('Unexpected preflight output directory')
            old = out.with_name('server-export.previous')
            if old.exists():raise ValueError('Previous export exists; inspect it before rebuilding')
            out.rename(old)
        logged(command,log,env)
        if args.command=='preflight':continue
        if target=='client':
            version = read(h/'instances.json')['instances'][m['instance']]['version']
            folder=h/'fork-versions'/version
        else:folder=out;version=None
        about = read(folder/'native-mods.json')
        verify_about(m, about, target)
        results[target] = {'directory':str(folder), 'version':version,
                           'files':runtime_files(folder, target, about)}
        results['manifest_sha256']=digest(p)
        write(p.parent/'artifacts.json',results)
    report = {'manifest':str(p),'check_only':args.command=='preflight','targets':args.target,'artifacts':results if args.command!='preflight' else {},'logs':str(p.parent)}
    if args.command != 'preflight':
        from fork_cache import purge_resource_cache, purge_launcher_cache, purge_workflow_cache
        report['resource_cache'] = purge_resource_cache(homes()[1])
        report['workflow_cache'] = purge_workflow_cache(ROOT, active_plan=p, workflow_locked=True)
        report['launcher_cache'] = purge_launcher_cache(
            h, workflow_locked=True, keep_versions=report['workflow_cache'].get('base_versions', ()))
    return report


def build_home(home):
    if os.environ.get('HORIZON_LAUNCHER_HOME'):return home
    config=Path(os.environ.get('XDG_CONFIG_HOME',Path.home()/'.config'))/'horizon-launcher-location/location.json'
    return Path(read(config).get('roots',{}).get('build',home))


def artifacts(path):
    p,m=load(path);a=read(p.parent/'artifacts.json')
    if a.get('manifest_sha256')!=digest(p):raise ValueError('No artifacts for this exact manifest')
    for key in ('client','server'):
        if key not in a:
            continue
        folder = Path(a[key]['directory'])
        about = read(folder/'native-mods.json')
        verify_about(m, about, key)
        if runtime_files(folder, key, about) != a[key]['files']:
            raise ValueError(f'{key} runtime files changed after the build')
    return p,m,a


def require_current_lunatic(manifest, checkout):
    from fork_curated import published_main
    if any(e['id'] == 'lunatic' for e in manifest['packages']):
        revision = manifest.get('lunatic_revision')
        current = published_main(checkout)
        if revision != current:
            raise ValueError(f'Frozen Lunatic revision {revision or "unknown"} differs from published {current}. Freeze a fresh manifest.')


def deploy(args):
    from fork import atomic_copy
    p,m,a=artifacts(args.manifest);h,game=homes()
    targets=['local','server']if args.target=='both'else[args.target]
    if 'local' in targets and 'client' not in a:raise ValueError('Build client artifacts first')
    if 'server' in targets:
        if args.apply:
            require_current_lunatic(m, workspace_root()/'Lunatic')
        if 'server' not in a:raise ValueError('Build server artifacts first')
        if not re.fullmatch(r'[A-Za-z0-9_.@-]+',args.host) or args.host.startswith('-'):raise ValueError('Invalid SSH host')
    result={'apply':args.apply,'revision':m['game_revision'],'native_mods':sorted(e['id']for e in m['packages']if e['kind']=='native')}
    if 'local'in targets:
        if 'client'not in a:raise ValueError('Build client artifacts first')
        if args.apply:
            backup=p.parent/('local-package-backup-'+dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ'))
            previous=[]
            def save(dest):
                target=backup/str(len(previous))
                existed=dest.exists()
                if existed:atomic_copy(dest,target)
                previous.append((dest,target,existed))
            for e in m['packages']:
                folder='horizon-native-mods'if e['kind']=='native'else'horizon-mods'
                save(game/folder/Path(e['path']).name)
            retired = game/'horizon-native-mods/corpse_native.hnmod'
            replace_corpse = any(e['id']=='gravestone_core' for e in m['packages'])
            if replace_corpse:save(retired)
            save(h/'instances.json');save(h/'settings.json')
            try:
                for e in m['packages']:
                    src=p.parent/e['path'];folder='horizon-native-mods'if e['kind']=='native'else'horizon-mods'
                    atomic_copy(src,game/folder/src.name)
                if replace_corpse:retired.unlink(missing_ok=True)
                instances=read(h/'instances.json');instances.setdefault('instances',{}).setdefault(m['instance'],{})['version']=a['client']['version'];write(h/'instances.json',instances)
                settings=read(h/'settings.json');settings['selected']=a['client']['version'];write(h/'settings.json',settings)
            except Exception:
                for dest,old,existed in previous:
                    if existed:shutil.copy2(old,dest)
                    else:dest.unlink(missing_ok=True)
                raise
            result['local_backup']=str(backup)
        result['local']={'version':a['client']['version'],'packages_match':all((game/('horizon-native-mods'if e['kind']=='native'else'horizon-mods')/Path(e['path']).name).exists()and digest(game/('horizon-native-mods'if e['kind']=='native'else'horizon-mods')/Path(e['path']).name)==e['sha256']for e in m['packages'])}
    if 'server'in targets:
        if 'server'not in a:raise ValueError('Build server artifacts first')
        if not re.fullmatch(r'[A-Za-z0-9_.@-]+',args.host)or args.host.startswith('-'):raise ValueError('Invalid SSH host')
        payload={'schema':SCHEMA,'root':args.server_dir,'service':args.service,'apply':args.apply,
                 'restart_occupied':args.restart_occupied,'files':{}}
        with tempfile.TemporaryDirectory(prefix='upload-', dir=p.parent) as temporary:
            staging = Path(temporary)
            server = a['server']
            runtime = Path(server['directory'])
            for name, sha in server['files'].items():
                dest = 'bin/'+name
                (staging/Path(dest).parent).mkdir(parents=True, exist_ok=True)
                shutil.copy2(runtime/name,staging/dest)
                payload['files'][dest] = sha
            for wrapper, executable in [('server', 'horizon-server'), ('engine', 'horizon-engine')]:
                dest = 'bin/' + wrapper
                entry = staging / dest
                entry.write_text('#!/bin/sh\nexec "$(dirname -- "$0")/' + executable + '" "$@"\n')
                entry.chmod(0o755)
                payload['files'][dest] = digest(entry)
            # Retain the channel publisher's root-level build record as well.
            shutil.copy2(runtime/'native-mods.json', staging/'native-mods.json')
            payload['files']['native-mods.json'] = server['files']['native-mods.json']
            for e in m['packages']:
                src=p.parent/e['path'];dest=('horizon-native-mods/'if e['kind']=='native'else'horizon-mods/')+src.name
                (staging/Path(dest).parent).mkdir(parents=True,exist_ok=True);shutil.copy2(src,staging/dest);payload['files'][dest]=e['sha256']
            from check_glibc import check_binaries
            with contextlib.redirect_stdout(sys.stderr):
                check_binaries(staging/name for name in payload['files']
                               if name in {'bin/horizon-engine','bin/horizon-server'}
                               or re.fullmatch(r'bin/[A-Za-z0-9_+.-]+\.so(?:\.[0-9]+)*', name))
            update_channel=None
            if args.apply:
                # Download inputs must be public and complete BEFORE changing a server.
                # Publication failure leaves the running server and client pointer alone.
                from fork_sync import stage_plan
                update_channel=stage_plan(p,m)
            remote=args.server_dir+'/.workflow-stage/'+digest(p)[:20]
            ssh=['ssh','--',args.host]
            if args.apply:
                subprocess.run(ssh+['mkdir -p -- '+shlex.quote(remote)],check=True)
            # tar over stdin avoids scp's remote-path parsing and shell interpolation.
            if args.apply:
                archive=subprocess.Popen(['tar','-C',str(staging),'-cf','-','.'],stdout=subprocess.PIPE)
                try:subprocess.run(ssh+['tar -xf - -C '+shlex.quote(remote)],stdin=archive.stdout,check=True)
                finally:
                    archive.stdout.close();code=archive.wait()
                if code:raise RuntimeError('Upload archive failed')
            runner=Path(__file__).with_name('fork_remote.py')
            payload['stage']=remote
            # The file manifest can exceed the OS limit for a single argument.
            bootstrap = 'import sys\nsys.argv.append(' + repr(json.dumps(payload)) + ')\n'
            command = bootstrap.encode() + runner.read_bytes()
            text=subprocess.check_output(ssh+['python3 -'],input=command,timeout=240)
            result['server']=json.loads(text)
            # Promote only inputs whose exact binaries and packages are now healthy.
            # A skipped deployment with different inputs must never advance clients.
            report=result['server']
            if args.apply and report.get('artifacts_match') and report.get('packages_match') and report.get('phase')=='running':
                from fork_sync import promote
                result['client_update_channel']=promote(*update_channel)
    if 'local' in targets:
        from fork_cache import purge_resource_cache
        result['resource_cache'] = purge_resource_cache(game, apply=args.apply)
    from fork_cache import purge_launcher_cache, purge_workflow_cache
    result['workflow_cache'] = purge_workflow_cache(ROOT, active_plan=p, apply=args.apply, workflow_locked=True)
    result['launcher_cache'] = purge_launcher_cache(
        h, apply=args.apply, workflow_locked=True, keep_versions=result['workflow_cache'].get('base_versions', ()))
    write(p.parent/'deployment.json',result)
    return result


def running_paths():
    paths=[]
    for proc in Path('/proc').glob('[0-9]*'):
        for name in ['exe','cwd']:
            try:paths.append((proc/name).resolve(strict=True))
            except OSError:pass
        try:
            for line in (proc/'maps').read_text().splitlines():
                if '/'in line:paths.append(Path('/'+line.split('/',1)[1].removesuffix(' (deleted)')))
        except OSError:pass
    return paths


def prune_candidates(home, exports, running):
    from fork_cache import _NATIVE_VERSION, _PLAYER_DATA, _RELEASE_VERSION
    keep={read(home/'settings.json').get('selected')}
    keep.update(v.get('version')for v in read(home/'instances.json').get('instances',{}).values())
    candidates=[];retained=[]
    versions=home/'fork-versions'
    for p in versions.iterdir() if versions.is_dir() and not versions.is_symlink() else ():
        # The shared launcher policy also runs automatically and owns these builds.
        if re.fullmatch(f'{_RELEASE_VERSION}|{_NATIVE_VERSION}',p.name):continue
        if p.is_symlink()or not p.is_dir()or not(p/'.installed').is_file():continue
        if p.name in keep or any(q.is_relative_to(p.resolve())for q in running):retained.append(p.name);continue
        if any((p/n).exists()or(p/n).is_symlink()for n in _PLAYER_DATA):retained.append(p.name);continue
        candidates.append(p)
    # Explicit export roots: remove executable copies only, never data/source/cache trees.
    for root in exports:
        root=root.resolve()
        if root.is_relative_to((home/'fork-versions').resolve()):raise ValueError('Managed builds must use reference-aware pruning')
        if not(root/'native-mods.json').is_file():raise ValueError('Export has no native metadata: '+str(root))
        if any(x in root.parts for x in ['target','cache','cargo','rust']):raise ValueError('Refusing cache cleanup')
        for n in ['horizon-client','horizon-engine','horizon-server','horizon-launcher','horizon-ctl','horizon-prepare']:
            for name in [n,n+'.exe']:
                p=root/name
                if p.is_file()and not p.is_symlink()and p not in running:candidates.append(p)
    return candidates,retained


def prune(args):
    if args.interrupted_only:
        from fork_cache import purge_launcher_cache
        return {'apply': args.apply, 'launcher_cache': purge_launcher_cache(
            apply=args.apply, workflow_locked=True, interrupted_only=True)}
    h,_=homes();items,kept=prune_candidates(h,args.exports,running_paths())
    removed=[]
    if args.apply:
        # Recalculate after preview, so a newly selected or launched build stays protected.
        for p in items:
            now,_=prune_candidates(h,args.exports,running_paths())
            if p not in now:continue
            if p.is_dir():shutil.rmtree(p)
            else:p.unlink()
            removed.append(p)
    from fork_cache import purge_resource_cache, purge_launcher_cache, purge_workflow_cache
    workflow=purge_workflow_cache(ROOT, apply=args.apply, workflow_locked=True)
    return {'apply':args.apply,'removed'if args.apply else'candidates':[str(p)for p in (removed if args.apply else items)],'kept':kept,
            'resource_cache':purge_resource_cache(homes()[1], apply=args.apply),
            'launcher_cache':purge_launcher_cache(
                h, apply=args.apply, workflow_locked=True, keep_versions=workflow.get('base_versions', ())),
            'workflow_cache':workflow}


def main(argv=None):
    import fcntl
    parser=argparse.ArgumentParser(description=__doc__)
    commands=parser.add_subparsers(dest='command',required=True)
    p=commands.add_parser('manifest');p.add_argument('--output',type=Path);p.add_argument('--revision');p.add_argument('--mods');p.add_argument('--base');p.add_argument('--packages-root',type=Path);p.add_argument('--instance',default='horizon')
    p.add_argument('--lunatic-ref',default='HEAD',help='Lunatic source commit that produced the frozen package')
    p.add_argument('--mods-ref',default='HEAD',help='Horizon Mods source commit that produced the frozen packages')
    for name in ['build-plan','preflight']:
        p=commands.add_parser(name);p.add_argument('manifest',type=Path);p.add_argument('--target',choices=['client','server','both'],default='both')
    p=commands.add_parser('deploy');p.add_argument('manifest',type=Path);p.add_argument('--target',choices=['local','server','both'],default='local');p.add_argument('--host',default='telemazer@74.91.116.171');p.add_argument('--server-dir',default='/home/telemazer/lunatic-server');p.add_argument('--service',default='lunatic-horizon.service');p.add_argument('--apply',action='store_true');p.add_argument('--restart-occupied',action='store_true',help='Restart the dedicated server even if players are online')
    p=commands.add_parser('prune-builds');p.add_argument('--exports',type=Path,action='append',default=[]);p.add_argument('--apply',action='store_true');p.add_argument('--dry-run',action='store_true');p.add_argument('--interrupted-only',action='store_true')
    args=parser.parse_args(argv)
    if args.command=='prune-builds'and args.apply and args.dry_run:parser.error('Choose --apply or --dry-run')
    if args.command=='deploy' and args.restart_occupied and args.target=='local':parser.error('--restart-occupied requires a server target')
    home,_=homes();home.mkdir(parents=True,exist_ok=True)
    with open(home/'.fork-workflow.lock','a')as lock:
        fcntl.flock(lock,fcntl.LOCK_EX)
        try:
            result={'manifest':snapshot,'build-plan':build,'preflight':build,'deploy':deploy,'prune-builds':prune}[args.command](args)
            print(json.dumps(result,sort_keys=True))
        except (OSError,ValueError,KeyError,RuntimeError,subprocess.SubprocessError,zipfile.BadZipFile)as error:
            print(json.dumps({'ok':False,'error':str(error)}),file=sys.stderr);return 1
    return 0


if __name__=='__main__':sys.exit(main())
