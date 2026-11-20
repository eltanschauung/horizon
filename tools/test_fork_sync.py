"""Focused source-channel tests; no live game, account or production paths."""
import json,tempfile,unittest,zipfile
from pathlib import Path
import fork_sync as sync
REVISION = "a" * 40
BASE = "v26.3"

def write_mod(path, mod_id):
    with zipfile.ZipFile(path, "w") as package:
        package.writestr("horizon-mod.toml", f"id='{mod_id}'\nname='{mod_id}'\nversion='1'\napi='0.2'\n"
                         f"capabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{REVISION}'\n"
                         f"mixins='{'b' * 40}'\n")
        package.writestr("Cargo.toml", "[package]\nname='fixture'\nversion='1.0.0'\nedition='2024'\n")
        package.writestr("src/lib.rs", "// fixture")

class ChannelTests(unittest.TestCase):
    def test_pins_assets_and_keeps_client_only_mods_optional(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);runtime=root/'runtime';runtime.mkdir();(runtime/'libopus-0.dll').write_bytes(b'codec');(runtime/'OPUS-LICENSE.txt').write_bytes(b'license')
            rows=[]
            for mod_id in ['lunatic','minimap']:
                file=root/(mod_id+'.hmod')
                write_mod(file, mod_id)
                rows.append({'id':mod_id,'version':'1','kind':'native','sha256':sync.sha(file),'file':file})
            m,files,tag=sync.prepare(REVISION,rows,root/'out',runtime,BASE)
            self.assertEqual(m['game_revision'],REVISION);self.assertEqual(m['base'],BASE)
            self.assertEqual(m['schema'],2);self.assertEqual(len(files),4)
            self.assertEqual(sum(n.endswith('.hmod') for n in files),2)
            by={p['id']:p for p in m['packages']};self.assertTrue(by['lunatic']['required']);self.assertFalse(by['minimap']['required'])
            self.assertEqual(by['libopus']['platform'],'windows-x86_64')
            self.assertTrue(all('/'+tag+'/' in p['url'] for p in m['packages']))
            again,_,_=sync.prepare(REVISION,list(reversed(rows)),root/'out2',runtime,BASE)
            self.assertEqual(m,again)
    def test_live_seed_rejects_bundle_different_from_running_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);folder=root/'horizon-native-mods';folder.mkdir();p=folder/'lunatic.hmod'
            write_mod(p, 'lunatic')
            record={'horizon':REVISION,'base':BASE,'target':'server','left_out':[],'mods':[{'id':'lunatic','version':'1','sha256':sync.native_digest(p)}]}
            (root/'native-mods.json').write_text(json.dumps(record))
            self.assertEqual(sync.inputs_from_live(root)[:2], (REVISION, BASE))
            with zipfile.ZipFile(p,'a') as z:z.writestr('changed.rs','source changed')
            with self.assertRaisesRegex(ValueError,'differs from compiled'):sync.inputs_from_live(root)
    def test_publisher_refuses_unsupported_package_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            package = root / "fixture.hmod"
            write_mod(package, "lunatic")
            row = dict(id="lunatic", version="1", kind="native", sha256=sync.sha(package), file=package)
            with self.assertRaisesRegex(ValueError, "not pinned"):
                sync.prepare("c" * 40, [row], root / "out", root, BASE)
            with self.assertRaisesRegex(ValueError, "duplicate"):
                sync.prepare(REVISION, [row, row], root / "out", root, BASE)
            with zipfile.ZipFile(package, "a") as old:
                old.writestr("horizon-native.json", "{}")
            with self.assertRaisesRegex(ValueError, "needs a Horizon SDK package"):
                sync.prepare(REVISION, [row], root / "out", root, BASE)

if __name__=='__main__':unittest.main()
