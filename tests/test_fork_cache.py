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
            self.assertEqual(len(preview['candidates']), 4)
            self.assertTrue(all(p.exists() for p in folders))
            with patch.object(cache, 'game_running', return_value=True):
                self.assertIn('deferred', cache.purge_resource_cache(home))
            with patch.object(cache, 'game_running', return_value=False):
                report = cache.purge_resource_cache(home)
            self.assertEqual(len(report['removed']), 4)
            self.assertEqual([p.exists() for p in folders], [True, False, False, False, False, True])
            self.assertTrue(unrelated.exists())
            self.assertTrue(external.exists())
            self.assertTrue((root/'registry-v1.bin').exists())

    def test_successful_plain_build_runs_all_cleanup_families(self):
        with patch.object(cache, 'purge_resource_cache', return_value={'removed': []}) as resources, \
             patch.object(cache, 'purge_launcher_cache', return_value={'removed': []}) as launcher, \
             patch.object(cache, 'purge_workflow_cache', return_value={'removed': []}) as workflow:
            cache.cleanup_after_build()
        resources.assert_called_once();launcher.assert_called_once();workflow.assert_called_once()

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
                self.assertEqual(len(preview['families']['server-builds']['candidates']),3)
                result = cache.purge_launcher_cache(home)
                again = cache.purge_launcher_cache(home, apply=False)
            self.assertEqual(len(result['removed']),11)
            self.assertTrue(all(not v['candidates'] for v in again['families'].values()))
            self.assertEqual(len(list((home/'fork-versions').iterdir())),2)
            self.assertTrue((target/'precious').exists())


class WorkflowCacheTests(unittest.TestCase):
    def setUp(self):
        import sys
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root/'horizon'
        self.repo.mkdir()
        self.launcher = self.root/'launcher'
        self.launcher.mkdir()
        self.addCleanup(patch.stopall)
        patch.object(sys, 'path', [str(Path(__file__).resolve().parents[1]/'tools')]+sys.path).start()
        patch('fork_workflow.homes', return_value=(self.launcher, self.root/'game')).start()
        patch('fork_workflow.running_paths', return_value=[]).start()
        patch.object(cache, '_native_compiler_running', return_value=False).start()

    def plan(self, name, age, targets=(), repo='horizon'):
        import json, hashlib
        path = self.root/repo/'.local'/name
        path.mkdir(parents=True)
        manifest = path/'manifest.json'
        manifest.write_text(json.dumps({'schema':1, 'game_revision':'a'*40,
            'packages':[{'path':'inputs/horizon-mods/test.hmod'}]}))
        for name in ('inputs', 'native-input', 'server-export'):
            child = path/name
            child.mkdir()
            (child/'generated').write_text('generated')
        (path/'STATUS.md').write_text('keep evidence')
        os.utime(manifest, (age, age))
        if targets:
            artifacts = {'manifest_sha256':hashlib.sha256(manifest.read_bytes()).hexdigest()}
            for target in targets:
                artifacts[target] = {'directory':str(path/'server-export'), 'files':{'generated':hashlib.sha256(b'generated').hexdigest()}}
            (path/'artifacts.json').write_text(json.dumps(artifacts))
            os.utime(path/'artifacts.json', (age+1, age+1))
        return path

    def test_all_old_exports_and_inputs_across_repositories(self):
        old = self.plan('nested/old', 100, ('client','server'))
        sibling = self.plan('old', 101, ('server',), repo='Lunatic')
        failed = self.plan('failed', 102)
        newest = self.plan('newest', 200, ('client','server'))
        superseded_pending = self.plan('superseded-pending', 250)
        pending = self.plan('pending', 300)
        # Shared Cargo/SDK trees and unowned source copies are never candidates.
        precious = self.repo/'target/compiled';precious.parent.mkdir();precious.write_text('keep')
        unknown = self.repo/'.local/unknown/server-export';unknown.mkdir(parents=True);(unknown/'source.rs').write_text('keep')
        preview = cache.purge_workflow_cache(self.repo, apply=False)
        self.assertTrue((old/'inputs').exists())
        self.assertEqual(len(preview['candidates']),12)
        result = cache.purge_workflow_cache(self.repo)
        self.assertEqual(len(result['removed']),12)
        for plan in (old, sibling, failed, superseded_pending):
            self.assertFalse((plan/'server-export').exists())
            self.assertFalse((plan/'inputs').exists())
            self.assertTrue((plan/'STATUS.md').exists())
        for plan in (newest,pending):self.assertTrue((plan/'inputs').exists())
        self.assertTrue(precious.exists());self.assertTrue((unknown/'source.rs').exists())
        self.assertEqual(cache.purge_workflow_cache(self.repo)['removed'],[])

    def test_per_target_active_running_player_data_and_symlinks(self):
        old = self.plan('old', 100, ('client','server'))
        active = self.plan('active', 105)
        running = self.plan('running', 106)
        world = self.plan('world', 107)
        (world/'server-export/saves').mkdir()
        linked = self.plan('linked', 108)
        shutil = __import__('shutil')
        shutil.rmtree(linked/'inputs')
        source = self.root/'source';source.mkdir();(source/'work.rs').write_text('keep')
        (linked/'inputs').symlink_to(source, target_is_directory=True)
        newest = self.plan('client', 200, ('client',))
        # Older server plan remains the newest successful server output.
        with patch('fork_workflow.running_paths', return_value=[running/'server-export/generated']):
            result = cache.purge_workflow_cache(self.repo, active_plan=active/'manifest.json')
        for plan in (old,active,running,newest):self.assertTrue((plan/'inputs').exists())
        self.assertTrue((world/'server-export/saves').exists())
        self.assertTrue((source/'work.rs').exists());self.assertTrue((linked/'inputs').is_symlink())
        self.assertNotIn(str(world/'server-export'),result['removed'])

    def test_previous_export_removed_only_after_verified_replacement(self):
        newest = self.plan('new', 200, ('server',))
        previous = newest/'server-export.previous';previous.mkdir();(previous/'engine').write_text('old')
        (newest/'server-export/generated').write_text('incomplete replacement')
        cache.purge_workflow_cache(self.repo)
        self.assertTrue(previous.exists())
        (newest/'server-export/generated').write_text('generated')
        cache.purge_workflow_cache(self.repo)
        self.assertFalse(previous.exists())

    def test_locks_compiler_and_invalid_manifest_are_safe(self):
        import fcntl
        old = self.plan('old',100,('client',))
        self.plan('new',200,('client',))
        with (self.launcher/'.fork-workflow.lock').open('a') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.assertIn('deferred',cache.purge_workflow_cache(self.repo))
        with patch.object(cache,'_native_compiler_running',return_value=True):
            self.assertIn('deferred',cache.purge_workflow_cache(self.repo))
        (old/'manifest.json').write_text('{bad')
        cache.purge_workflow_cache(self.repo)
        self.assertTrue((old/'inputs').exists())


    def test_partial_installations_are_removed_but_running_ones_survive(self):
        parent = self.launcher/'fork-native/server-builds'
        stale = parent/'.native-abcd-0000.partial';stale.mkdir(parents=True)
        live = parent/'.native-abcd-0001.partial';live.mkdir()
        with patch('fork_workflow.running_paths',return_value=[live/'engine']):
            result = cache.purge_launcher_cache(self.launcher)
        self.assertFalse(stale.exists());self.assertTrue(live.exists())
        self.assertIn(str(stale), result['removed'])
