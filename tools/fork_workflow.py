#!/usr/bin/env python3
"""Frozen native build inputs, compact deployment reports and conservative pruning."""
import argparse
import datetime as dt
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


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


def safe_id(value):
    if not re.fullmatch(r'[a-z0-9_]+', value):
        raise ValueError('Invalid package id: '+value)
    return value


def package(path, native):
    with zipfile.ZipFile(path) as z:
        if native:
            info = json.loads(z.read('horizon-native.json'))
        else:
            info = tomllib.loads(z.read('horizon-mod.toml').decode())
    return {'id': safe_id(info['id']), 'version': info['version'], 'sha256': digest(path)}


def unpack(source, destination):
    with zipfile.ZipFile(source) as z:
        for entry in z.infolist():
            p = Path(entry.filename)
            if p.is_absolute() or '..' in p.parts or (entry.external_attr >> 16) & 0o170000 == 0o120000:
                raise ValueError('Unsafe source bundle member: '+entry.filename)
        if sum(e.file_size for e in z.infolist()) > 5_000_000_000:
            raise ValueError('Source bundle exceeds 5 GB')
        z.extractall(destination)


def snapshot(args):
    from fork import fork_launcher
    h, game = homes()
    revision = git('rev-parse', args.revision or 'HEAD')
    if not re.fullmatch(r'[a-f0-9]{40}', revision):
        raise ValueError('Expected a complete game commit')
    launcher = Path(fork_launcher()).resolve()
    provenance = json.loads(subprocess.check_output([launcher, 'fork-info'], text=True))
    if provenance.get('dirty') or not provenance.get('revision') or 'cached-preflight' not in provenance.get('capabilities', []):
        raise ValueError('Install the committed workflow-capable fork launcher first')
    stamp = dt.datetime.now(dt.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    out = (args.output or ROOT/'.local/build-plans'/stamp/'manifest.json').resolve()
    if out.exists() or (out.parent/'inputs').exists():
        raise ValueError('Choose a fresh manifest directory')
    entries = []
    seen = set()
    wanted = set(args.mods.split(',')) if args.mods else None
    for folder, suffix, native in [('horizon-native-mods','hnmod',True),('horizon-mods','hmod',False)]:
        for src in sorted((game/folder).glob('*.'+suffix)):
            info = package(src, native)
            if native and wanted is not None and info['id'] not in wanted:
                continue
            key = (native, info['id'])
            if key in seen:raise ValueError('Duplicate package id: '+info['id'])
            seen.add(key)
            dest = out.parent/'inputs'/folder/(info['id']+'.'+suffix)
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dest)
            entries.append(dict(info, kind='native' if native else 'content', path=str(dest.relative_to(out.parent))))
    ids = {e['id'] for e in entries if e['kind']=='native'}
    if not ids or (wanted is not None and wanted != ids):
        raise ValueError('Native package selection is empty or contains missing ids')
    src = git('show', revision+':launcher/src/rust_toolchain.rs')
    toolchain = re.search(r'pub const VERSION: &str = "([^"]+)"', src).group(1)
    shutil.copy2(launcher, out.parent/'inputs/launcher')
    manifest = {'schema':1, 'created_utc':stamp, 'game_revision':revision,
                'launcher_revision':provenance['revision'], 'launcher_tree':provenance['launcher_tree'],
                'launcher_sha256':digest(launcher), 'launcher_version':subprocess.check_output([launcher,'--version'],text=True).strip(),
                'mods_revision':subprocess.check_output(['git','-C',ROOT.parent/'horizon-mods','rev-parse','HEAD'],text=True).strip(),
                'toolchain':toolchain, 'profile':args.profile, 'base':args.base or read(h/'instances.json').get('instances',{}).get(args.instance,{}).get('version') or read(h/'settings.json').get('selected'),
                'instance':args.instance, 'packages':entries}
    if not manifest['base']:
        raise ValueError('No installed base version; supply --base')
    write(out, manifest)
    return {'manifest':str(out),'manifest_sha256':digest(out),'revision':revision,'native_mods':sorted(ids)}


def load(path):
    path = Path(path).resolve()
    m = read(path)
    if m.get('schema')!=1 or not re.fullmatch(r'[a-f0-9]{40}',m.get('game_revision','')):
        raise ValueError('Invalid workflow manifest')
    for e in m['packages']:
        safe_id(e['id'])
        src = (path.parent/e['path']).resolve()
        if not src.is_relative_to(path.parent) or digest(src)!=e['sha256']:
            raise ValueError('Snapshot changed: '+e['path'])
    if digest(path.parent/'inputs/launcher')!=m['launcher_sha256']:
        raise ValueError('Frozen launcher changed')
    return path,m


def verify_about(m, about):
    expected = {e['id']:e['version'] for e in m['packages'] if e['kind']=='native'}
    actual = {e['id']:e['version'] for e in about['mods']}
    if about['horizon']!=m['game_revision'] or expected!=actual or about.get('left_out'):
        raise ValueError('Build does not match pinned revision and complete native mod set')
    if about['rust']!=m['toolchain'] or about['profile']!=m['profile']:
        raise ValueError('Build compiler/profile differs from the manifest')
    return sorted(actual)


def logged(command, log, env=None):
    with open(log,'w') as stream:
        result = subprocess.run(command, env=env, stdout=stream, stderr=subprocess.STDOUT)
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
    env['HORIZON_FORK_REVISION']=m['game_revision']
    if args.command=='preflight':env['HORIZON_FORK_PREFLIGHT']='1'
    else:env.pop('HORIZON_FORK_PREFLIGHT',None)
    source = p.parent/'native-input'
    if source.exists():shutil.rmtree(source)
    for e in m['packages']:
        if e['kind']=='native':unpack(p.parent/e['path'],source/e['id'])
    results = read(p.parent/'artifacts.json')
    # Separate targets are sequential: the launcher owns one reusable staging workspace.
    for target in (['client','server'] if args.target=='both' else [args.target]):
        log = p.parent/(args.command+'-'+target+'.log')
        out = p.parent/('server-check' if args.command=='preflight' else 'server-export')
        command = [str(p.parent/'inputs/launcher'),'native-build',str(source),'--version',m['base'],
                   '--trust-native-code','--allow-version-mismatch','--profile',m['profile']]
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
        about=read(folder/'native-mods.json');verify_about(m,about)
        files=['native-mods.json','horizon-client' if target=='client' else 'horizon-engine']
        if target=='server':
            # Native export carries a base supervisor. Build this revision's supervisor
            # with the SAME staged workspace, managed compiler and native target cache.
            build_supervisor(p,m,env,log.with_name('supervisor.log'))
            cache=build_home(h)/'fork-native'
            shutil.copy2(cache/'target'/('fast' if m['profile']=='performance' else 'release')/'horizon-server',folder/'horizon-server')
            files+=['horizon-server']
        results[target]={'directory':str(folder),'version':version,'files':{f:digest(folder/f)for f in files}}
        results['manifest_sha256']=digest(p)
        write(p.parent/'artifacts.json',results)
    return {'manifest':str(p),'check_only':args.command=='preflight','targets':args.target,'artifacts':results if args.command!='preflight' else {},'logs':str(p.parent)}


def build_home(home):
    if os.environ.get('HORIZON_LAUNCHER_HOME'):return home
    config=Path(os.environ.get('XDG_CONFIG_HOME',Path.home()/'.config'))/'horizon-launcher-location/location.json'
    return Path(read(config).get('roots',{}).get('build',home))


def build_supervisor(p,m,env,log):
    h,_=homes();root=build_home(h);cache=root/'fork-native';stage=cache/'stage'
    compiler=root/'rust'/(m['toolchain']+'-x86_64-unknown-linux-gnu')/'bin'
    if not (compiler/'cargo').is_file():raise ValueError('Expected managed Linux x86-64 toolchain missing')
    clean={k:v for k,v in env.items() if not k.startswith(('CARGO_','RUST','HORIZON_FORK_PREFLIGHT'))}
    clean.update(PATH=str(compiler)+os.pathsep+env.get('PATH',''),RUSTC=str(compiler/'rustc'),CARGO_HOME=str(root/'cargo'),CARGO_TARGET_DIR=str(cache/'target'))
    command=[str(compiler/'cargo'),'build','--manifest-path',str(stage/'Cargo.toml'),'--locked','--profile','fast' if m['profile']=='performance' else 'release','-p','horizon-server','-j','2']
    # cwd / excludes home Cargo configuration, matching the managed build.
    with open(log,'w')as stream:subprocess.run(command,cwd='/',env=clean,stdout=stream,stderr=subprocess.STDOUT,check=True)


def artifacts(path):
    p,m=load(path);a=read(p.parent/'artifacts.json')
    if a.get('manifest_sha256')!=digest(p):raise ValueError('No artifacts for this exact manifest')
    for key in ['client','server']:
        if key not in a:continue
        folder=Path(a[key]['directory']);verify_about(m,read(folder/'native-mods.json'))
        for f,sha in a[key]['files'].items():
            if digest(folder/f)!=sha:raise ValueError('Artifact changed: '+str(folder/f))
    if 'client'in a and 'server'in a:
        def mods(k):return read(Path(a[k]['directory'])/'native-mods.json')['mods']
        if sorted(mods('client'),key=lambda x:x['id'])!=sorted(mods('server'),key=lambda x:x['id']):raise ValueError('Client/server compiled mod fingerprints differ')
    return p,m,a


def deploy(args):
    from fork import atomic_copy
    p,m,a=artifacts(args.manifest);h,game=homes()
    targets=['local','server']if args.target=='both'else[args.target]
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
            save(h/'instances.json');save(h/'settings.json')
            try:
                for e in m['packages']:
                    src=p.parent/e['path'];folder='horizon-native-mods'if e['kind']=='native'else'horizon-mods'
                    atomic_copy(src,game/folder/src.name)
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
        payload={'schema':1,'root':args.server_dir,'service':args.service,'apply':args.apply,'files':{}}
        staging=p.parent/'upload';staging.mkdir(exist_ok=True)
        for name in ['horizon-engine','horizon-server','native-mods.json']:
            dest=('bin/'if name!='native-mods.json'else'')+name;src=Path(a['server']['directory'])/name
            (staging/Path(dest).parent).mkdir(parents=True,exist_ok=True);shutil.copy2(src,staging/dest);payload['files'][dest]=digest(src)
        for e in m['packages']:
            src=p.parent/e['path'];dest=('horizon-native-mods/'if e['kind']=='native'else'horizon-mods/')+src.name
            (staging/Path(dest).parent).mkdir(parents=True,exist_ok=True);shutil.copy2(src,staging/dest);payload['files'][dest]=e['sha256']
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
        runner=ROOT/'tools/fork_remote.py'
        payload['stage']=remote
        cmd='python3 - '+shlex.quote(json.dumps(payload))
        text=subprocess.check_output(ssh+[cmd],input=runner.read_bytes(),timeout=240)
        result['server']=json.loads(text)
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
    keep={read(home/'settings.json').get('selected')}
    keep.update(v.get('version')for v in read(home/'instances.json').get('instances',{}).values())
    candidates=[];retained=[]
    for p in (home/'fork-versions').iterdir():
        if p.is_symlink()or not p.is_dir()or not(p/'.installed').is_file():continue
        if p.name in keep or any(q.is_relative_to(p.resolve())for q in running):retained.append(p.name);continue
        if any((p/n).exists()for n in ['saves','world','playerdata','options.txt']):retained.append(p.name);continue
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
    h,_=homes();items,kept=prune_candidates(h,args.exports,running_paths())
    if args.apply:
        # Recalculate after preview, so a newly selected or launched build stays protected.
        for p in items:
            now,_=prune_candidates(h,args.exports,running_paths())
            if p not in now:continue
            if p.is_dir():shutil.rmtree(p)
            else:p.unlink()
    return {'apply':args.apply,'removed'if args.apply else'candidates':[str(p)for p in items],'kept':kept,'caches_preserved':True}


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__)
    commands=parser.add_subparsers(dest='command',required=True)
    p=commands.add_parser('manifest');p.add_argument('--output',type=Path);p.add_argument('--revision');p.add_argument('--mods');p.add_argument('--base');p.add_argument('--instance',default='horizon');p.add_argument('--profile',choices=['performance','release'],default='performance')
    for name in ['build-plan','preflight']:
        p=commands.add_parser(name);p.add_argument('manifest',type=Path);p.add_argument('--target',choices=['client','server','both'],default='both')
    p=commands.add_parser('deploy');p.add_argument('manifest',type=Path);p.add_argument('--target',choices=['local','server','both'],default='local');p.add_argument('--host',default='telemazer@74.91.116.171');p.add_argument('--server-dir',default='/home/telemazer/lunatic-server');p.add_argument('--service',default='lunatic-horizon.service');p.add_argument('--apply',action='store_true')
    p=commands.add_parser('prune-builds');p.add_argument('--exports',type=Path,action='append',default=[]);p.add_argument('--apply',action='store_true');p.add_argument('--dry-run',action='store_true')
    args=parser.parse_args(argv)
    if args.command=='prune-builds'and args.apply and args.dry_run:parser.error('Choose --apply or --dry-run')
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
