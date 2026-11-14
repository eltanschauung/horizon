"""Focused source-channel tests; no live game, account or production paths."""
import hashlib,json,tempfile,unittest,zipfile
from pathlib import Path
import fork_sync as sync
class ChannelTests(unittest.TestCase):
    def test_pins_assets_and_keeps_client_only_mods_optional(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);runtime=root/'runtime';runtime.mkdir();(runtime/'libopus-0.dll').write_bytes(b'codec');(runtime/'OPUS-LICENSE.txt').write_bytes(b'license')
            rows=[]
            for mod_id in ['lunatic','minimap']:
                file=root/(mod_id+'.hnmod')
                with zipfile.ZipFile(file,'w') as z:z.writestr('horizon-native.json',json.dumps({'id':mod_id,'version':'1'}))
                rows.append({'id':mod_id,'version':'1','kind':'native','sha256':sync.sha(file),'file':file})
            m,files,tag=sync.prepare('a'*40,rows,root/'out',runtime)
            self.assertEqual(m['game_revision'],'a'*40);self.assertEqual(len(files),4)
            by={p['id']:p for p in m['packages']};self.assertTrue(by['lunatic']['required']);self.assertFalse(by['minimap']['required'])
            self.assertEqual(by['libopus']['platform'],'windows-x86_64')
            self.assertTrue(all('/'+tag+'/' in p['url'] for p in m['packages']))
            again,_,_=sync.prepare('a'*40,list(reversed(rows)),root/'out2',runtime)
            self.assertEqual(m,again)
    def test_live_seed_rejects_bundle_different_from_running_build(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=Path(tmp);folder=root/'horizon-native-mods';folder.mkdir();p=folder/'lunatic.hnmod'
            with zipfile.ZipFile(p,'w') as z:z.writestr('horizon-native.json',json.dumps({'id':'lunatic','version':'1'}))
            record={'horizon':'a'*40,'mods':[{'id':'lunatic','version':'1','sha256':sync.native_digest(p)}]}
            (root/'native-mods.json').write_text(json.dumps(record));sync.inputs_from_live(root)
            with zipfile.ZipFile(p,'a') as z:z.writestr('changed.rs','source changed')
            with self.assertRaisesRegex(ValueError,'differs from compiled'):sync.inputs_from_live(root)
if __name__=='__main__':unittest.main()
