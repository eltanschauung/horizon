"""Workflow failure/safety checks; no real services, worlds or launcher homes."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile
import fork_cache
import fork_remote as remote
import fork_workflow as w


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)

    def test_snapshot_tampering_stops_before_build_or_deploy(self):
        p=self.root/'manifest.json';launcher=self.root/'inputs/launcher';launcher.parent.mkdir();launcher.write_bytes(b'launcher')
        pkg=self.root/'inputs/mod.hmod';pkg.write_bytes(b'original')
        w.write(p,{'schema':w.SCHEMA,'game_revision':'a'*40,'launcher_sha256':w.digest(launcher),'packages':[{'id':'mod','path':'inputs/mod.hmod','sha256':w.digest(pkg)}]})
        w.load(p);pkg.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'Snapshot changed'):w.load(p)

    def test_partial_build_and_wrong_revision_are_rejected(self):
        m={'game_revision':'a'*40,'toolchain':'1.99.1','profile':'optimized','base':'sdk-base','packages':[{'kind':'native','id':'lunatic','version':'1','source_sha256':'stable'}]}
        about={'horizon':'a'*40,'rust':'1.99.1','profile':'optimized','base':'sdk-base','mods':[{'id':'lunatic','version':'1','sha256':'stable'}],'left_out':[]}
        self.assertEqual(w.verify_about(m,about),['lunatic'])
        for key,value in [('mods',[]),('left_out',[{'id':'lunatic'}]),('horizon','b'*40),('rust','other')]:
            with self.subTest(key=key),self.assertRaises(ValueError):w.verify_about(m,dict(about,**{key:value}))

    def test_distribution_requires_every_frozen_default(self):
        with self.assertRaisesRegex(ValueError, 'Missing native SDK packages:.*lunatic'):
            w.collect_sdk_packages(self.root, 'a'*40)
        distribution = w.read(w.ROOT/'tools/fork-distribution.json')
        self.assertEqual(len(distribution['native']), 32)
        self.assertEqual(len(distribution['content']), 19)
        self.assertIn('emojiful', distribution['native'])
        self.assertNotIn('emojiful_twemoji', distribution['content'])
        self.assertTrue(set(distribution['native']).isdisjoint(distribution['content']))

    def test_runtime_libraries_and_package_trees_are_in_artifact_hashes(self):
        source = self.root/'horizon-native-mods/lunatic'
        source.mkdir(parents=True)
        names = ['horizon-engine','horizon-server','native-mods.json','libmod_lunatic.so',
                 'horizon-native-mods/lunatic/horizon-mod.toml']
        for name in names:
            (self.root/name).write_bytes(name.encode())
        about = {'mods':[{'id':'lunatic'}],
                 'libraries':[{'file':'libmod_lunatic.so', 'sha256':w.digest(self.root/'libmod_lunatic.so')}]}
        hashes = w.runtime_files(self.root, 'server', about)
        self.assertEqual(set(hashes), set(names))
        self.assertTrue(all(remote.allowed('bin/'+name) for name in names))
        self.assertFalse(remote.allowed('bin/horizon-native-mods/lunatic/../../playerdata'))
        (self.root/'libmod_lunatic.so').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'library missing or changed'):
            w.runtime_files(self.root, 'server', about)

    def test_sources_cannot_escape_extraction_folder(self):
        z=self.root/'bad.hnmod'
        with zipfile.ZipFile(z,'w')as f:f.writestr('../escape','bad')
        with self.assertRaises(ValueError):w.unpack(z,self.root/'input')
        self.assertFalse((self.root/'escape').exists())

    def test_prune_keeps_referenced_running_and_player_data(self):
        for name in ['selected','instance','running','with-world','old']:
            d=self.root/'fork-versions'/name;d.mkdir(parents=True);(d/'.installed').touch()
        (self.root/'fork-versions/with-world/saves').mkdir()
        w.write(self.root/'settings.json',{'selected':'selected'})
        w.write(self.root/'instances.json',{'instances':{'other':{'version':'instance'}}})
        items,kept=w.prune_candidates(self.root,[],[self.root/'fork-versions/running/horizon-client'])
        self.assertEqual([p.name for p in items],['old']);self.assertEqual(set(kept),{'selected','instance','running','with-world'})

    def test_prune_refuses_target_cache_even_if_it_looks_like_export(self):
        (self.root/'fork-versions').mkdir();cache=self.root/'target/export';cache.mkdir(parents=True);(cache/'native-mods.json').write_text('{}')
        with self.assertRaisesRegex(ValueError,'cache'):w.prune_candidates(self.root,[cache],[])

    def test_export_option_cannot_bypass_selected_build_protection(self):
        p=self.root/'fork-versions/current';p.mkdir(parents=True);(p/'.installed').touch();(p/'native-mods.json').write_text('{}')
        w.write(self.root/'settings.json',{'selected':'current'})
        with self.assertRaisesRegex(ValueError,'reference-aware'):w.prune_candidates(self.root,[p],[])

    def test_release_cleanup_keeps_newest_and_build_bases(self):
        versions = self.root/'fork-versions'
        for name in ('v0.3.10', 'v0.3.9', 'v0.3.8', 'v0.3.7', 'v0.3.6'):
            path = versions/name
            path.mkdir(parents=True)
            (path/'.installed').touch()
        w.write(versions/'native-dead-beef/native-mods.json', {'base': 'v0.3.8'})
        w.write(self.root/'settings.json', {'selected': 'v0.3.6'})
        with patch.dict(os.environ, {'HORIZON_LAUNCHER_HOME': str(self.root)}):
            result = fork_cache.purge_launcher_cache(self.root, apply=False, keep_versions=['v0.3.7'])
        releases = result['families']['release-versions']
        self.assertEqual(releases['candidates'], [str(versions/'v0.3.9')])
        self.assertTrue((versions/'v0.3.9').exists())

    def test_interrupted_cleanup_removes_only_idle_partial_outputs(self):
        native = self.root/'fork-native'
        partial = native/'source'/('.'+'a'*40+'.partial')
        abandoned = native/'stage.new'
        fresh = native/'mods.incoming'
        working = native/'stage'
        protected = native/'mixins.incoming'
        for path in (partial, abandoned, fresh, working, protected/'saves'):
            path.mkdir(parents=True)
        for path in (partial, abandoned, protected):
            os.utime(path, (1, 1))
        with patch.object(w, 'build_home', return_value=self.root), patch.object(
                fork_cache, '_native_compiler_running', return_value=False):
            result = fork_cache.purge_launcher_cache(self.root, interrupted_only=True)
        self.assertEqual(set(result['removed']), {str(partial), str(abandoned)})
        self.assertTrue(all(p.exists() for p in (fresh, working, protected)))

    def test_windows_scratch_cleanup_requires_old_outputs_and_both_cargo_locks(self):
        cache = self.root/'fork-native/target-windows'
        host = cache/'fast'
        target = cache/'x86_64-pc-windows-gnu/fast'
        for profile in (host, target):
            profile.mkdir(parents=True)
            (profile/'.cargo-lock').touch()
        (cache/'CACHEDIR.TAG').write_text('Signature: 8a477f597d28d172789f06886806bc55\n')
        artifact = target/'horizon-client.exe'
        artifact.touch()
        self.assertEqual(fork_cache._purge_retired_windows_output(cache, [], apply=True)['removed'], [])
        stale = time.time() - 4 * 24 * 60 * 60
        os.utime(artifact, (stale, stale))
        with (host/'.cargo-lock').open('rb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            self.assertEqual(fork_cache._purge_retired_windows_output(cache, [], apply=True)['removed'], [])
        preview = fork_cache._purge_retired_windows_output(cache, [], apply=False)
        self.assertEqual(preview['candidates'], [str(artifact)])
        self.assertTrue(artifact.exists())
        result = fork_cache._purge_retired_windows_output(cache, [], apply=True)
        self.assertEqual(result['removed'], [str(artifact)])
        self.assertFalse(artifact.exists())
        self.assertTrue((host/'.cargo-lock').is_file())
        self.assertTrue((target/'.cargo-lock').is_file())

    def test_new_server_content_defaults_preserve_explicit_switches(self):
        config = self.root/'horizon-mods.txt'
        config.write_bytes(b'mod.disabled:false')
        packages = []
        for name, side in [('disabled', 'both'), ('temples', 'both'), ('emoji', 'client')]:
            package = self.root/(name+'.hmod')
            with zipfile.ZipFile(package, 'w') as bundle:
                bundle.writestr('horizon-mod.toml', f'id = "{name}"\nside = "{side}"\n')
            packages.append(package)
        settings, enabled = remote.content_defaults(config, packages)
        self.assertEqual(settings, b'mod.disabled:false\nmod.temples:true\n')
        self.assertEqual(enabled, ['temples'])
        remote.install_mod_settings(config, settings)
        self.assertEqual(remote.content_defaults(config, packages), (settings, []))

    def test_server_rollout_requires_current_lunatic_source(self):
        manifest = {'packages': [{'id': 'lunatic'}]}
        with patch('fork_curated.published_main', return_value='current'):
            with self.assertRaisesRegex(ValueError, 'Freeze a fresh manifest'):
                w.require_current_lunatic(manifest, self.root)
            with self.assertRaisesRegex(ValueError, 'stale differs'):
                w.require_current_lunatic(dict(manifest, lunatic_revision='stale'), self.root)
            w.require_current_lunatic(dict(manifest, lunatic_revision='current'), self.root)

    def test_player_count_must_be_known(self):
        for status in [{'phase':'starting'},{'phase':'running','sample':None}]:
            with self.assertRaises(RuntimeError):remote.players(status)

    def payload(self):
        r=self.root;stage=r/'.workflow-stage/test';(stage/'bin').mkdir(parents=True);(r/'bin').mkdir()
        files={}
        for name in ['engine','server','horizon-engine','horizon-server']:
            (r/'bin'/name).write_bytes(b'old-'+name.encode());(stage/'bin'/name).write_bytes(b'new-'+name.encode());files['bin/'+name]=w.digest(stage/'bin'/name)
        return {'schema':w.SCHEMA,'root':str(r),'stage':str(stage),'service':'test.service','apply':True,'files':files}

    @staticmethod
    def healthy(players=None):return {'phase':'running','crashes':0,'sample':{'game':{'players':players or[],'tps':20}}}

    def test_online_player_skips_without_stopping_service(self):
        p=self.payload()
        with patch.object(remote.subprocess,'check_output',return_value=json.dumps(self.healthy(['player'])).encode()),patch.object(remote.subprocess,'run')as run:
            result=remote.execute(p)
        self.assertEqual(result['skipped'],'players_online');run.assert_not_called()
        self.assertEqual((self.root/'bin/horizon-engine').read_bytes(),b'old-horizon-engine')

    def test_failed_restart_restores_old_binaries(self):
        p=self.payload();states=[self.healthy(),self.healthy(),dict(self.healthy(),crashes=1)]
        with patch.object(remote.subprocess,'check_output',side_effect=[json.dumps(s).encode()for s in states]),patch.object(remote.subprocess,'run')as run:
            with self.assertRaisesRegex(RuntimeError,'rolled back'):remote.execute(p)
        self.assertEqual((self.root/'bin/horizon-engine').read_bytes(),b'old-horizon-engine')
        self.assertEqual((self.root/'bin/server').read_bytes(),b'old-server')
        self.assertEqual((self.root/'bin/engine').read_bytes(),b'old-engine')
        self.assertEqual([c.args[0][3]for c in run.call_args_list if c.args[0][0]=='sudo'],['stop','start','stop','start'])

    def test_unknown_destination_never_written(self):
        p=self.payload();p['files']['../world/level.dat']='abc'
        with self.assertRaises(ValueError):remote.execute(p)


if __name__=='__main__':unittest.main()
