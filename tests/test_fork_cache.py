import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('fork_cache', Path(__file__).resolve().parents[1]/'tools/fork_cache.py')
cache = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cache)


class ResourceCacheTests(unittest.TestCase):
    def test_combined_limit_preview_and_unrelated_data(self):
        with tempfile.TemporaryDirectory() as temp:
            home = Path(temp)
            root = home/'.horizon-cache'
            folders = []
            for n in range(6):
                family = 'mod-content' if n % 2 else 'resourcepacks'
                folder = root/family/f'{n:016x}'
                folder.mkdir(parents=True)
                (folder/'asset').write_text('generated')
                os.utime(folder, (100+n, 100+n))
                folders.append(folder)
            (root/'registry-v1.bin').write_text('keep')
            unrelated = root/'mod-content/user-files'
            unrelated.mkdir()
            external = home/'source'; external.mkdir()
            (root/'mod-content/aaaaaaaaaaaaaaaa').symlink_to(external, target_is_directory=True)
            # Reusing an old snapshot promotes it above newly created ones.
            stamp = folders[0]/'horizon-mod-content.stamp'
            stamp.write_text('stamp'); os.utime(stamp, (200, 200))
            preview = cache.purge_resource_cache(home, apply=False)
            self.assertEqual(len(preview['candidates']), 3)
            self.assertTrue(all(p.exists() for p in folders))
            with patch.object(cache, 'game_running', return_value=True):
                self.assertIn('deferred', cache.purge_resource_cache(home))
            with patch.object(cache, 'game_running', return_value=False):
                report = cache.purge_resource_cache(home)
            self.assertEqual(len(report['removed']), 3)
            self.assertEqual([p.exists() for p in folders], [True, False, False, False, True, True])
            self.assertTrue(unrelated.exists())
            self.assertTrue(external.exists())
            self.assertTrue((root/'registry-v1.bin').exists())

    def test_launcher_limits_preserve_references_and_compiler_cache(self):
        import sys
        import json
        tools = str(Path(__file__).resolve().parents[1]/'tools')
        with tempfile.TemporaryDirectory() as temp, patch.object(sys, 'path', [tools]+sys.path):
            home = Path(temp)
            native = home/'fork-native'
            for family, width in [('source',40), ('patched',64), ('server-builds',0), ('versions',0)]:
                parent = home/'fork-versions' if family=='versions' else native/family
                for i in range(4):
                    name = f'{i:0{width}x}' if width else f'native-abcd-{i:04x}'
                    p = parent/name; p.mkdir(parents=True); (p/'artifact').write_text('cache')
                    os.utime(p, (100+i,100+i))
            target = native/'target';target.mkdir();(target/'precious').write_text('compiled')
            (home/'settings.json').write_text(json.dumps({'selected':'native-abcd-0000'}))
            with patch('fork_workflow.running_paths', return_value=[]):
                preview = cache.purge_launcher_cache(home, apply=False)
                self.assertEqual(len(preview['families']['server-builds']['candidates']),2)
                result = cache.purge_launcher_cache(home)
                again = cache.purge_launcher_cache(home, apply=False)
            self.assertEqual(len(result['removed']),7)
            self.assertTrue(all(not v['candidates'] for v in again['families'].values()))
            self.assertEqual(len(list((home/'fork-versions').iterdir())),3)
            self.assertTrue((target/'precious').exists())
