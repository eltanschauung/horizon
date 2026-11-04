#!/usr/bin/env python3
"""Isolated visual proof: transient effects cannot move the minimap or footer.

Run under xvfb-run with a client containing the native minimap. --mod may
supply content packages; --character selects an installed character if needed.
All worlds, options, mod settings, sockets and logs live in temporary homes.
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
import tomllib
import zipfile


def until(observe, accept, seconds=30):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            last = observe()
            if accept(last):
                return last
        except (OSError, ValueError, subprocess.SubprocessError):
            pass
        time.sleep(.1)
    raise AssertionError(f"Timeout: {last}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("game", "ctl", "resources", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--mod", type=Path, action="append", default=[])
    parser.add_argument("--character")
    args = parser.parse_args()
    for name in ("game", "ctl", "resources", "output"):
        setattr(args, name, getattr(args, name).resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    children = []
    with tempfile.TemporaryDirectory(prefix="horizon-hud-layout-") as temp:
        root = Path(temp)
        def home(name):
            folder = root / name
            folder.mkdir()
            mods = folder / "horizon-mods"
            mods.mkdir()
            enabled = ["mod.minimap:true"]
            for package in args.mod:
                shutil.copy2(package, mods / package.name)
                with zipfile.ZipFile(package) as archive:
                    meta = tomllib.loads(archive.read("horizon-mod.toml").decode())
                enabled.append(f"mod.{meta['id']}:true")
            (folder / "horizon-mods.txt").write_text("\n".join(enabled) + "\n")
            (folder / "options.txt").write_text("renderDistance:2\nsimulationDistance:2\nautoJump:false\npauseOnLostFocus:false\nguiScale:2\ntutorialStep:none\n")
            return folder
        def launch(folder, arguments, label):
            env = dict(os.environ, HORIZON_HOME=str(folder), HORIZON_LAUNCHER_HOME=str(folder / "launcher"), XDG_CONFIG_HOME=str(folder / "config"), XDG_DATA_HOME=str(folder / "data"), XDG_CACHE_HOME=str(folder / "cache"), HORIZON_NO_AUDIO="1")
            with (args.output / (label + ".log")).open("w") as log:
                child = subprocess.Popen([str(args.game), "--resources", str(args.resources), "--game-dir", str(folder), "--home", str(folder), *arguments], cwd=folder, env=env, stdin=subprocess.PIPE, stdout=log, stderr=log)
            children.append(child)
            return child
        def ctl(*words):
            reply = subprocess.run([str(args.ctl), "--socket", str(client_home / "control.sock"), "--timeout", "10", *words], capture_output=True, text=True, timeout=15)
            if reply.returncode:
                raise ValueError(reply.stdout + reply.stderr)
            return json.loads(reply.stdout)
        def console(command):
            server.stdin.write((command + "\n").encode())
            server.stdin.flush()
        def shot(name):
            path = args.output / (name + ".png")
            ctl("screenshot", str(path))
            return subprocess.check_output(["ffmpeg", "-v", "error", "-i", str(path), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        def pixel(image, x, y):
            i = (y * 800 + x) * 4
            return image[i:i+4]
        def frame(image, rect):
            x, y, size = rect
            # Compare only the fixed outline, not asynchronously composed terrain.
            points = [(x+i, y) for i in range(size)] + [(x+i, y+size-1) for i in range(size)] + [(x, y+i) for i in range(size)] + [(x+size-1, y+i) for i in range(size)]
            return b"".join(pixel(image, x, y) for x, y in points)
        def region(image, x, y, size=48):
            return b"".join(image[((y+j)*800+x)*4:((y+j)*800+x+size)*4] for j in range(size))
        server_home = home("server")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        settings = {"server-ip":"127.0.0.1", "server-port":port, "online-mode":"false", "enforce-secure-profile":"false", "white-list":"false", "view-distance":2, "simulation-distance":2, "level-type":"minecraft:flat", "generator-settings":json.dumps({"layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:dirt","height":2},{"block":"minecraft:grass_block","height":1}],"biome":"minecraft:plains","features":False,"lakes":False,"structure_overrides":[]})}
        (server_home / "server.properties").write_text("".join(f"{k}={v}\n" for k,v in settings.items()))
        (server_home / "eula.txt").write_text("eula=true\n")
        proof = []
        try:
            server = launch(server_home, ["--dedicated", "--nogui"], "server")
            until(lambda: socket.create_connection(("127.0.0.1", port), 1).close(), lambda _: True, 90)
            console("gamerule spawn_mobs false")
            console("gamerule spawn_monsters false")
            client_home = home("client")
            if args.character:
                (client_home / "character.json").write_text(json.dumps({"character":args.character}))
            config = client_home / "horizon-mods/.horizon-mod-data/minimap"
            config.mkdir(parents=True)
            for key,value in {"corner":"top_right","size":"64","shape":"square","outline":"stone","rotate":"false","show_coordinates":"true"}.items():
                (config / key).write_text(value)
            client = launch(client_home, ["--server",f"127.0.0.1:{port}","--name","HudProof","--control","--control-socket",str(client_home / "control.sock"),"--width","800","--height","600"], "client")
            until(lambda: ctl("status"), lambda s:(s.get("server") or {}).get("loaded",False), 90)
            console("tp @a 0.5 -60 0.5 0 0")
            console("clear @a")
            time.sleep(3)
            cases = [("top_right",size,coords) for size in (64,96,128) for coords in (True,False)] + [(corner,64,True) for corner in ("top_left","bottom_left","bottom_right")]
            for corner,size,coords in cases:
                label = f"{corner}-{size}-coords-{coords}"
                console("effect clear @a")
                until(lambda:ctl("state")["player"]["effects"],lambda e:not e)
                for key,value in {"corner":corner,"size":str(size),"show_coordinates":str(coords).lower()}.items():
                    (config / key).write_text(value)
                time.sleep(.5)
                first = shot(label + "-none")
                assert len(first) == 800*600*4, "Unexpected screenshot resolution"
                x = 4 if corner.endswith("left") else 800-4-size*2
                y = 4 if corner.startswith("top") else 600-(size+2+(9 if coords else 0))*2
                rect = (x,y,size*2)
                # Verify that the minimap is present at the intended stable anchor.
                assert pixel(first,x,y)[:3] == bytes(3), (label,pixel(first,x,y))
                console("effect give @a minecraft:speed 60 0 true")
                until(lambda:ctl("state")["player"]["effects"],lambda e:len(e)==1)
                beneficial = shot(label + "-beneficial")
                assert frame(first,rect) == frame(beneficial,rect), label
                console("effect give @a minecraft:weakness 60 0 true")
                until(lambda:ctl("state")["player"]["effects"],lambda e:len(e)==2)
                mixed = shot(label + "-mixed")
                assert frame(first,rect) == frame(mixed,rect), label
                top = (size+4+(9 if coords else 0))*2 if corner=="top_right" else 2
                assert region(first,750,top) != region(beneficial,750,top), label
                assert region(beneficial,750,top) == region(mixed,750,top), label
                assert region(first,750,top+52) != region(mixed,750,top+52), label
                proof.append({"corner":corner,"size":size,"coordinates":coords,"map_stable":True,"effect_top_pixels":top})
                print("PASS:",label,flush=True)
            (config / "corner").write_text("top_right")
            time.sleep(.4)
            visible = shot("hud-visible")
            ctl("key","f1","tap")
            time.sleep(.3)
            hidden = shot("hud-hidden")
            assert frame(visible,(668,4,128)) != frame(hidden,(668,4,128))
            ctl("key","f1","tap")
            ctl("key","e","tap")
            until(lambda:ctl("status")["screen"],lambda s:s=="inventory")
            inventory = shot("inventory")
            assert frame(visible,(668,4,128)) == frame(inventory,(668,4,128))
            result = {"passed":True,"cases":proof,"f1_hides_hud":True,"inventory_map_stable":True}
            (args.output / "result.json").write_text(json.dumps(result,indent=2)+"\n")
            print(json.dumps(result),flush=True)
        finally:
            for child in reversed(children):
                if child.poll() is None:
                    try:
                        child.stdin.write(b"stop\n")
                        child.stdin.flush()
                        child.wait(timeout=3)
                    except (BrokenPipeError,OSError,subprocess.TimeoutExpired):
                        child.terminate()
            for child in children:
                try: child.wait(timeout=10)
                except subprocess.TimeoutExpired: child.kill();child.wait()


if __name__ == "__main__":
    main()
