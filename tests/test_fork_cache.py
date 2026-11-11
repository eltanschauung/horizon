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
