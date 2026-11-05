#!/usr/bin/env python3
"""Verify deferred character joins through a real dedicated server and clients.

All homes/worlds are temporary; pass a Lunatic-enabled native client build.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time


def wait(read, predicate, timeout=60):
    end = time.monotonic() + timeout
    last = None
    while time.monotonic() < end:
        try:
            last = read()
            if predicate(last):
                return last
        except (OSError, ValueError, subprocess.SubprocessError):
            pass
        time.sleep(.2)
    raise AssertionError(f'Timed out; last observation: {last!r}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('game', 'ctl', 'resources', 'mod', 'output'):
        parser.add_argument('--' + key, type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    children = []
    with tempfile.TemporaryDirectory(prefix='horizon-character-joins-') as temp:
        root = Path(temp)
        def home(name):
            d = root / name
            d.mkdir()
            (d / 'horizon-mods').mkdir()
            shutil.copy2(args.mod, d / 'horizon-mods/lunatic.hmod')
            (d / 'horizon-mods.txt').write_text('mod.lunatic:true\n')
            (d / 'options.txt').write_text('renderDistance:2\nsimulationDistance:2\npauseOnLostFocus:false\n')
            return d
        def launch(d, extra, label):
            env = dict(os.environ, HORIZON_HOME=str(d), HORIZON_LAUNCHER_HOME=str(d/'launcher'), XDG_CONFIG_HOME=str(d/'config'), XDG_DATA_HOME=str(d/'data'), XDG_CACHE_HOME=str(d/'cache'), HORIZON_NO_AUDIO='1', HORIZON_TELEMETRY_URL='off')
            with (args.output/(label+'.log')).open('w') as log:
                child = subprocess.Popen([str(args.game), '--resources', str(args.resources), '--game-dir', str(d), '--home', str(d), *extra], cwd=d, env=env, stdin=subprocess.PIPE, stdout=log, stderr=log)
            children.append(child)
            return child
        def ctl(d, *words):
            result = subprocess.run([str(args.ctl), '--socket', str(d/'c.sock'), '--timeout', '10', *words], capture_output=True, text=True, timeout=15, check=True)
            return json.loads(result.stdout)
        def connect(d, account, character, label):
            (d/'character.json').write_text(json.dumps({'character':'lunatic:'+character}))
            p = launch(d, ['--server', f'127.0.0.1:{port}', '--name', account, '--control', '--control-socket', str(d/'c.sock'), '--width', '640', '--height', '480'], label)
            wait(lambda:ctl(d, 'status'), lambda s:(s.get('server') or {}).get('loaded'), 90)
            time.sleep(.5)
            return p
        def chat(d):
            return ctl(d, 'chat-log', '100')['chat']
        def joined(d):
            return [s for s in chat(d) if 'joined the game' in s]
        server_home = home('server')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
        props = {'server-ip':'127.0.0.1', 'server-port':port, 'online-mode':'false', 'white-list':'false', 'enforce-secure-profile':'false', 'view-distance':2, 'simulation-distance':2, 'level-type':'minecraft:flat', 'level-seed':123, 'generator-settings':json.dumps({'layers':[{'block':'minecraft:bedrock','height':1},{'block':'minecraft:dirt','height':2},{'block':'minecraft:grass_block','height':1}], 'biome':'minecraft:plains', 'features':False, 'lakes':False, 'structure_overrides':[]})}
        (server_home/'server.properties').write_text(''.join(f'{k}={v}\n' for k,v in props.items()))
        (server_home/'eula.txt').write_text('eula=true\n')
        try:
            launch(server_home, ['--dedicated', '--nogui'], 'server')
            wait(lambda:socket.create_connection(('127.0.0.1',port),1).close(), lambda _:True, 90)
            observer = home('observer'); connect(observer, 'JoinObserver', 'hakurei_reimu', 'observer')
            visitor = home('visitor'); process = connect(visitor, 'JoinVisitor', 'komeiji_satori', 'first-join')
            try:
                wait(lambda:joined(observer), lambda lines:len(lines)==1, 12)
            except AssertionError:
                (args.output/'result.json').write_text(json.dumps({'passed':False, 'stage':'first_join', 'observer_chat':chat(observer)}, indent=2))
                raise
            assert joined(observer)==['Komeiji Satori joined the game'], joined(observer)
            # Let another account reserve Cirno before the reconnect switches to it.
            holder = home('holder'); connect(holder, 'JoinHolder', 'cirno', 'holder')
            wait(lambda:joined(observer), lambda lines:len(lines)==2, 12)
            process.terminate(); process.wait(timeout=10)
            wait(lambda:chat(observer), lambda lines:any('Komeiji Satori left the game' in s for s in lines), 12)
            connect(visitor, 'JoinVisitor', 'cirno', 'changed-reconnect')
            lines = wait(lambda:joined(observer), lambda lines:len(lines)==3, 12)
            assert lines[0]=='Cirno (B) joined the game', lines
            time.sleep(1)
            assert joined(observer)==lines, 'Duplicate announcement after idle'
            assert not any('JoinVisitor' in s or 'JoinHolder' in s for s in chat(observer)), chat(observer)
            evidence = {'passed':True, 'joins':lines, 'account_names_hidden':True, 'no_duplicates':True}
            (args.output/'result.json').write_text(json.dumps(evidence, indent=2)+'\n')
            print(json.dumps(evidence), flush=True)
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
            for child in children:
                try:child.wait(timeout=10)
                except subprocess.TimeoutExpired:child.kill(); child.wait()


if __name__ == '__main__':
    main()
