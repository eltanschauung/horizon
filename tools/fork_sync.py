#!/usr/bin/env python3
"""Publish immutable client inputs matching a healthy server, without client prebuilds."""
from pathlib import Path
import argparse,base64,contextlib,hashlib,json,shutil,struct,subprocess,sys,tempfile,tomllib,zipfile
REPO='eltanschauung/horizon'
BRANCH='server-sync'
# Presentation-only mods may be switched off without changing the server's game.
OPTIONAL={'chatheads','classic_music','eating_drinking_animation','eatinganimation','firstpersonmodel_horizon','invis_armor','jade','justenoughitems','minimap','zoom','voicechat_server','dynamiclights','lambdynamiclights_horizon','lambdynamiclights'}

def sha(path):
    with Path(path).open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def identity(path,native):
    with zipfile.ZipFile(path) as z:
        return json.loads(z.read('horizon-native.json')) if native else tomllib.loads(z.read('horizon-mod.toml').decode())

def native_digest(path):
    h=hashlib.sha256()
    with zipfile.ZipFile(path) as z:
        for n in sorted(n for n in z.namelist() if not n.endswith('/')):
            b=z.read(n);h.update(n.encode());h.update(struct.pack('<Q',len(b)));h.update(b)
    return h.hexdigest()

def inputs_from_live(root):
    """Read only: prove bundles are what the active engine was compiled with."""
    about=json.loads((root/'native-mods.json').read_text());built={m['id']:m for m in about['mods']}
    entries=[];seen=set()
    for folder,kind,suffix in [('horizon-native-mods','native','hnmod'),('horizon-mods','content','hmod')]:
        for path in sorted((root/folder).glob('*.'+suffix)):
            info=identity(path,kind=='native');mod_id=info['id']
            if kind=='native':
                if built.get(mod_id,{}).get('sha256')!=native_digest(path):raise ValueError('Live bundle differs from compiled server: '+mod_id)
                seen.add(mod_id)
            entries.append({'id':mod_id,'version':info['version'],'kind':kind,'sha256':sha(path),'file':path})
    if seen!=set(built):raise ValueError('Live native bundle set is incomplete')
    return about['horizon'],entries

def inputs_from_plan(path,manifest):
    entries=[]
    for p in manifest['packages']:
        file=path.parent/p['path']
        if sha(file)!=p['sha256']:raise ValueError('Frozen update input changed: '+p['id'])
        info=identity(file,p['kind']=='native')
        if (info['id'],info['version'])!=(p['id'],p['version']):raise ValueError('Frozen update identity changed')
        entries.append(dict(p,file=file))
    return manifest['game_revision'],entries

def prepare(revision,entries,output,runtime,base='v0.3.5'):
    """Pure staging: no GitHub write, server change, Minecraft files or user data."""
    if len(revision)!=40 or any(c not in '0123456789abcdef' for c in revision):raise ValueError('Source must be an exact commit')
    output.mkdir(parents=True,exist_ok=True)
    rows=[];assets={}
    for e in sorted(entries,key=lambda e:(e['kind'],e['id'])):
        file=Path(e['file']);kind=e['kind'];info=identity(file,kind=='native')
        if info['id']!=e['id'] or info['version']!=e['version'] or sha(file)!=e['sha256']:raise ValueError('Update input changed')
        ext='hnmod' if kind=='native' else 'hmod';name=e['id']+'-'+e['sha256']+'.'+ext
        dest=output/name
        if not dest.exists():shutil.copy2(file,dest)
        assets[name]=dest
        rows.append({'id':e['id'],'version':e['version'],'kind':kind,'sha256':e['sha256'],'size':file.stat().st_size,'required':e['id'] not in OPTIONAL,'platform':None,'asset':name})
    for mod_id,name in [('libopus','libopus-0.dll'),('opus_license','OPUS-LICENSE.txt')]:
        src=runtime/name
        if not src.is_file():raise ValueError('Windows runtime is required: '+name)
        digest=sha(src);asset=mod_id+'-'+digest+'.'+('dll' if mod_id=='libopus' else 'txt');dest=output/asset
        if not dest.exists():shutil.copy2(src,dest)
        assets[asset]=dest
        rows.append({'id':mod_id,'version':'1.6.1-2','kind':'runtime','sha256':digest,'size':src.stat().st_size,'required':True,'platform':'windows-x86_64','asset':asset})
    core={'schema':1,'game_revision':revision,'base':base,'packages':rows}
    generation=hashlib.sha256(json.dumps(core,sort_keys=True,separators=(',',':')).encode()).hexdigest()
    tag='sync-'+generation[:20]
    manifest=dict(core,generation=generation)
    for p in manifest['packages']:p['url']=f'https://github.com/{REPO}/releases/download/{tag}/'+p.pop('asset')
    (output/'manifest.json').write_text(json.dumps(manifest,indent=2,sort_keys=True)+'\n')
    return manifest,assets,tag

def gh(*args,body=None):
    cmd=['gh',*args]
    if body is None:return subprocess.check_output(cmd,stderr=subprocess.PIPE,text=True)
    with tempfile.NamedTemporaryFile('w',suffix='.json') as f:
        json.dump(body,f);f.flush()
        return subprocess.check_output(cmd+['--input',f.name],stderr=subprocess.PIPE,text=True)

def upload_assets(manifest,assets,tag):
    # This channel holds source/content inputs only. End users receive no
    # platform game releases and build through the launcher's existing cache.
    probe=subprocess.run(['gh','release','view',tag,'--repo',REPO,'--json','tagName'],capture_output=True,text=True)
    if probe.returncode:
        gh('release','create',tag,'--repo',REPO,'--target',manifest['game_revision'],'--prerelease','--latest=false','--title','Server update '+manifest['generation'][:12],'--notes','Immutable launcher update inputs for the dedicated server. No game binaries, Minecraft assets or player files.')
    remote=json.loads(gh('api',f'repos/{REPO}/releases/tags/{tag}'));have={a['name']:a for a in remote['assets']}
    for name,file in assets.items():
        expected=sha(file)
        if name in have:
            if have[name].get('digest')=='sha256:'+expected:continue
            with tempfile.TemporaryDirectory() as tmp:
                gh('release','download',tag,'--repo',REPO,'--pattern',name,'--dir',tmp)
                if sha(Path(tmp)/name)!=expected:raise ValueError('Immutable published update asset differs: '+name)
        else:gh('release','upload',tag,str(file),'--repo',REPO)

def promote(manifest,output):
    # Promote the pointer ONLY after all immutable assets exist. Reusing this
    # function after a failed publication is safe and never overwrites an asset.
    check=subprocess.run(['gh','api',f'repos/{REPO}/git/ref/heads/{BRANCH}'],capture_output=True,text=True)
    if check.returncode:
        head=json.loads(gh('api',f'repos/{REPO}/git/ref/heads/main'))['object']['sha']
        gh('api',f'repos/{REPO}/git/refs',body={'ref':'refs/heads/'+BRANCH,'sha':head})
    check=subprocess.run(['gh','api',f'repos/{REPO}/contents/manifest.json?ref={BRANCH}'],capture_output=True,text=True)
    body={'message':'Match launcher updates to the healthy dedicated server','branch':BRANCH,'content':base64.b64encode((output/'manifest.json').read_bytes()).decode()}
    if check.returncode==0:body['sha']=json.loads(check.stdout)['sha']
    gh('api','--method','PUT',f'repos/{REPO}/contents/manifest.json',body=body)
    return {'generation':manifest['generation'],'revision':manifest['game_revision'],'packages':len(manifest['packages']),'channel':f'https://raw.githubusercontent.com/{REPO}/{BRANCH}/manifest.json'}

def publish(manifest,assets,tag,output):
    upload_assets(manifest,assets,tag)
    return promote(manifest,output)

def stage_plan(path,manifest):
    from windows_runtime import prepare as prepare_runtime
    out=path.parent/'server-sync';runtime=out/'runtime'
    with contextlib.redirect_stdout(sys.stderr):prepare_runtime(runtime,Path.home()/'.cache/horizon-windows-runtime')
    revision,entries=inputs_from_plan(path,manifest);m,assets,tag=prepare(revision,entries,out,runtime)
    upload_assets(m,assets,tag)
    return m,out

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--live-root',type=Path,required=True);p.add_argument('--runtime',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--publish',action='store_true');a=p.parse_args()
    revision,entries=inputs_from_live(a.live_root);m,assets,tag=prepare(revision,entries,a.output,a.runtime)
    # Recheck active files immediately before promotion so a concurrent deployment
    # cannot publish a half-old manifest. This command never stops the server.
    if a.publish:
        upload_assets(m,assets,tag)
        revision2,entries2=inputs_from_live(a.live_root)
        if revision2!=revision or [(e['id'],e['sha256']) for e in entries2]!=[(e['id'],e['sha256']) for e in entries]:raise ValueError('Active server changed during publication')
        print(json.dumps(promote(m,a.output)))
    else:print(json.dumps({'generation':m['generation'],'revision':revision,'packages':len(m['packages']),'published':False}))
if __name__=='__main__':main()
