#!/usr/bin/env python3
"""Remote half of fork_workflow deploy; sent to Python over SSH, never installed."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time


def sha(path):
    with open(path,'rb')as f:return hashlib.file_digest(f,'sha256').hexdigest()


def allowed(name):
    return name in {'bin/horizon-engine','bin/horizon-server','native-mods.json'} or bool(re.fullmatch(r'horizon-(?:native-mods/[a-z0-9_]+\.hnmod|mods/[a-z0-9_]+\.hmod)',name))


def players(status):
    game=(status.get('sample')or{}).get('game')
    if status.get('phase')!='running' or not isinstance(game,dict) or not isinstance(game.get('players'),list):
        raise RuntimeError('Cannot safely determine online players')
    return game['players']


def execute(payload):
    root=Path(payload['root']);stage=Path(payload['stage']);service=payload['service']
    if not root.is_absolute()or not stage.is_relative_to(root/'.workflow-stage')or not re.fullmatch(r'[a-zA-Z0-9_.@-]+\.service',service):raise ValueError('Invalid deployment destination')
    if not all(allowed(n)for n in payload['files']):raise ValueError('Invalid deployment file')
    def status():return json.loads(subprocess.check_output([root/'bin/server','--dir',root,'status','--json']))
    before=status();online=players(before)
    matches={n:(root/n).is_file()and sha(root/n)==value for n,value in payload['files'].items()}
    report={'apply':payload['apply'],'players':len(online),'artifacts_match':all(matches.values()),'packages_match':all(v for n,v in matches.items()if n.startswith(('horizon-mods/','horizon-native-mods/'))),'changed':[n for n,same in matches.items()if not same],'phase':before['phase']}
    if not payload['apply']:return report
    if online:return dict(report,skipped='players_online')
    for name,value in payload['files'].items():
        if (stage/name).is_symlink()or sha(stage/name)!=value:raise ValueError('Uploaded artifact mismatch: '+name)
        if (root/name).is_symlink():raise ValueError('Refusing symlink destination: '+name)
    # Check loader/library compatibility before stopping the working service.
    env=dict(os.environ,HORIZON_HOME=str(stage/'probe'),HORIZON_NO_AUDIO='1')
    loader=root/'lib/ld-linux-x86-64.so.2'
    for name,flag in [('horizon-engine','--help'),('horizon-server','--version')]:
        path=stage/'bin'/name;path.chmod(0o755)
        cmd=[str(loader),'--library-path',str(root/'lib'),str(path)]if loader.exists()else[str(path)]
        subprocess.run(cmd+[flag],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,check=True,timeout=30)
    backup=root/'backups'/('workflow-'+datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S%fZ'))
    extras=[]
    for folder,suffix in [('horizon-native-mods','*.hnmod'),('horizon-mods','*.hmod')]:
        extras.extend(str(p.relative_to(root))for p in(root/folder).glob(suffix)if str(p.relative_to(root))not in payload['files'])
    names=list(payload['files'])+extras
    existed={name:(root/name).exists()for name in names}
    for name in names:
        if existed[name]:
            dest=backup/name;dest.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(root/name,dest)
    # Recheck immediately before stopping: someone may have joined during the upload/probes.
    if players(status()):return dict(report,skipped='players_online')
    def control(verb):subprocess.run(['sudo','-n','systemctl',verb,service],check=True,stdout=subprocess.DEVNULL)
    control('stop')
    try:
        for name in payload['files']:
            dest=root/name;dest.parent.mkdir(parents=True,exist_ok=True);os.replace(stage/name,dest)
        for name in extras:(root/name).unlink()
        control('start')
        deadline=time.monotonic()+120
        while time.monotonic()<deadline:
            current=status()
            if current.get('crashes',0):raise RuntimeError('Updated engine crashed')
            game=(current.get('sample')or{}).get('game')or{}
            if current.get('phase')=='running'and game.get('tps',0)>=18:
                if any(sha(root/n)!=v for n,v in payload['files'].items()):raise RuntimeError('Post-deploy package mismatch')
                return dict(report,phase='running',packages_match=True,artifacts_match=True,tps=game['tps'],crashes=current['crashes'],backup=str(backup))
            time.sleep(2)
        raise RuntimeError('Updated server did not become healthy within 120 seconds')
    except Exception as error:
        control('stop')
        for name in names:
            if existed[name]:shutil.copy2(backup/name,root/name)
            else:(root/name).unlink(missing_ok=True)
        control('start')
        raise RuntimeError('Deployment rolled back: '+str(error))from error


if __name__=='__main__':
    try:print(json.dumps(execute(json.loads(sys.argv[1])),sort_keys=True))
    except Exception as error:
        print(json.dumps({'ok':False,'error':str(error)}),file=sys.stderr);sys.exit(1)
