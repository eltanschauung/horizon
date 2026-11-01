#!/usr/bin/env python3
"""Exercise two real clients against a private, offline dedicated test server.
All worlds, settings and character choices are isolated in a temporary directory.
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


def until(observe, accept, seconds=180):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            last = observe()
            if accept(last):
                return last
        except (OSError, ValueError, subprocess.SubprocessError):
            pass
        time.sleep(0.5)
    raise AssertionError(f'Timed out; last observation: {last}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('game', 'ctl', 'resources', 'mod'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--worldspawn', action='store_true', help='Also test the operator worldspawn command')
    args = parser.parse_args()
    args.game, args.ctl, args.resources, args.mod = (p.resolve() for p in (args.game, args.ctl, args.resources, args.mod))
    with tempfile.TemporaryDirectory(prefix='lunatic-network-test-') as directory:
        root = Path(directory)
        children = []
        logs = []
        env = dict(os.environ, HORIZON_TELEMETRY_URL='off', HORIZON_NO_AUDIO='1')
        def home(name):
            path = root / name
            path.mkdir(exist_ok=True)
            (path / 'horizon-mods').mkdir(exist_ok=True)
            shutil.copy2(args.mod, path / 'horizon-mods/lunatic.hmod')
            (path / 'horizon-mods.txt').write_text('mod.lunatic:true\n')
            (path / 'options.txt').write_text('version:5023\nrenderDistance:2\nsimulationDistance:2\n')
            return path
        server = home('server')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        settings = {'server-ip': '127.0.0.1', 'server-port': port, 'online-mode': 'false',
                    'enforce-secure-profile': 'false', 'white-list': 'false', 'view-distance': 2,
                    'simulation-distance': 2, 'level-type': 'minecraft:flat', 'level-seed': 123,
                    'generator-settings': json.dumps({'layers': [{'block':'minecraft:bedrock','height':1},
                        {'block':'minecraft:dirt','height':2},{'block':'minecraft:grass_block','height':1}],
                        'biome':'minecraft:plains','features':False,'lakes':False,'structure_overrides':[]})}
        (server / 'server.properties').write_text(''.join(f'{k}={v}\n' for k,v in settings.items()))
        (server / 'eula.txt').write_text('eula=true\n')
        def launch(path, extra):
            log = open(path / 'test-process.log', 'w+')
            logs.append(log)
            proc = subprocess.Popen([str(args.game), '--resources', str(args.resources), *extra],
                cwd=path, env=dict(env,HORIZON_HOME=str(path)), stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT)
            children.append(proc)
            return proc
        def ctl(path, *words):
            r = subprocess.run([str(args.ctl), '--socket', str(path/'c.sock'), '--timeout', '10', *words],
                capture_output=True, text=True, timeout=15)
            if r.returncode:
                raise ValueError(r.stderr + r.stdout)
            return json.loads(r.stdout.strip().splitlines()[-1])
        def client(name, character):
            path = home(name)
            (path/'character.json').write_text(json.dumps({'character':character}))
            proc = launch(path, ['--headless','--server',f'127.0.0.1:{port}', '--name',name,
                '--control','--control-socket',str(path/'c.sock')])
            until(lambda: ctl(path,'status'), lambda s: (s.get('server') or {}).get('joined',False))
            return path, proc
        try:
            srv = launch(server, ['--dedicated','--nogui'])
            until(lambda: socket.create_connection(('127.0.0.1',port),1).close(),lambda _:True)
            alice, a = client('Alice','lunatic:hakurei_reimu')
            until(lambda:ctl(alice,'status'),lambda s:len(s['server']['characters'])==1)
            bob, b = client('Bob','lunatic:kirisame_marisa')
            for path in (alice,bob):
                state=until(lambda:ctl(path,'status'),lambda s:len(s['server']['characters'])==2)
                assert {c['id'] for c in state['server']['characters'].values()} == {'lunatic:hakurei_reimu','lunatic:kirisame_marisa'}
                assert {c['color'] for c in state['server']['characters'].values()} == {'red','yellow'}
            ctl(bob,'chat','character-sync-proof')
            until(lambda:ctl(alice,'status'),lambda s:any('Kirisame Marisa : character-sync-proof' in line for line in s['server']['chat']))
            until(lambda:ctl(alice,'status'),lambda s:'Kirisame Marisa' in (s.get('name_tags') or []))
            if args.worldspawn:
                spawn_y = round(ctl(bob,'status')['player']['position'][1])
                def console(command):
                    srv.stdin.write((command+'\n').encode());srv.stdin.flush()
                def position(path): return ctl(path,'status')['player']['position']
                def near_spawn(state):
                    p=state['player']['position']
                    return state['server']['dimension']=='minecraft:overworld' and all(abs(a-b)<0.05 for a,b in zip(p,[2.5,spawn_y,-2.5]))
                console('op Alice')
                until(lambda:(server/'test-process.log').read_text(),lambda text:'Made Alice a server operator' in text)
                console('gamemode creative Alice')
                console(f'setworldspawn 2 {spawn_y} -3 90 0')
                until(lambda:(server/'test-process.log').read_text(),lambda text:f'world spawn point to 2, {spawn_y}, -3' in text)
                console(f'spawnpoint Alice 10 {spawn_y} 10')
                console(f'tp Alice 25.5 {spawn_y} 25.5')
                until(lambda:position(alice),lambda p:abs(p[0]-25.5)<0.05)
                before=position(bob)
                ctl(bob,'chat','/worldspawn')
                time.sleep(1)
                after=position(bob)
                assert abs(before[0]-after[0])<0.05 and abs(before[2]-after[2])<0.05, 'Non-operator was teleported'
                ctl(alice,'chat','/worldspawn')
                state=until(lambda:ctl(alice,'status'),near_spawn)
                assert abs(state['yaw']-90)<0.05
                console('execute in minecraft:the_nether run tp Alice 0 80 0')
                until(lambda:ctl(alice,'status'),lambda state:state['server']['dimension']=='minecraft:the_nether')
                ctl(alice,'chat','/worldspawn')
                until(lambda:ctl(alice,'status'),near_spawn)
                print('PASS: operator worldspawn, non-operator denied, bed ignored, centering/facing and Nether return',flush=True)
            ctl(bob,'quit'); b.wait(timeout=30)
            until(lambda:ctl(alice,'status'),lambda s:len(s['server']['characters'])==1)
            bob,b=client('Bob','lunatic:cirno')
            until(lambda:ctl(alice,'status'),lambda s:any(c['id']=='lunatic:cirno' for c in s['server']['characters'].values()))
            saved=json.loads((server/'character-selections.json').read_text())
            assert set(saved.values()) == {'lunatic:hakurei_reimu','lunatic:cirno'}
            print('PASS: two clients, late join, names/colors, chat, nametag, disconnect, changed choice, reconnect and persisted UUID choices')
        finally:
            for proc in children:
                if proc.poll() is None:
                    try:
                        proc.stdin.write(b'stop\n');proc.stdin.flush()
                    except (BrokenPipeError,OSError):pass
            for proc in children:
                try:proc.wait(timeout=20)
                except subprocess.TimeoutExpired:proc.terminate();proc.wait(timeout=10)
            for log in logs:log.close()
            # Preserve diagnostics outside the game/account folders.
            dest = Path('/tmp/lunatic-network-test-logs')
            dest.mkdir(exist_ok=True)
            for path in root.glob('*/test-process.log'):
                shutil.copy2(path,dest/(path.parent.name+'.log'))


if __name__ == '__main__':
    main()
