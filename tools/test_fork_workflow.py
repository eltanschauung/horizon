"""Workflow failure/safety checks; no real services, worlds or launcher homes."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile
import fork_remote as remote
import fork_workflow as w


class WorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)

    def test_snapshot_tampering_stops_before_build_or_deploy(self):
        p=self.root/'manifest.json';launcher=self.root/'inputs/launcher';launcher.parent.mkdir();launcher.write_bytes(b'launcher')
        pkg=self.root/'inputs/mod.hnmod';pkg.write_bytes(b'original')
        w.write(p,{'schema':1,'game_revision':'a'*40,'launcher_sha256':w.digest(launcher),'packages':[{'id':'mod','path':'inputs/mod.hnmod','sha256':w.digest(pkg)}]})
        w.load(p);pkg.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError,'Snapshot changed'):w.load(p)

    def test_partial_build_and_wrong_revision_are_rejected(self):
        m={'game_revision':'a'*40,'toolchain':'1.99.1','profile':'performance','packages':[{'kind':'native','id':'lunatic','version':'1'}]}
        about={'horizon':'a'*40,'rust':'1.99.1','profile':'performance','mods':[{'id':'lunatic','version':'1'}],'left_out':[]}
        self.assertEqual(w.verify_about(m,about),['lunatic'])
        for key,value in [('mods',[]),('left_out',[{'id':'lunatic'}]),('horizon','b'*40),('rust','other')]:
            with self.subTest(key=key),self.assertRaises(ValueError):w.verify_about(m,dict(about,**{key:value}))

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

    def test_player_count_must_be_known(self):
        for status in [{'phase':'starting'},{'phase':'running','sample':None}]:
            with self.assertRaises(RuntimeError):remote.players(status)

    def payload(self):
        r=self.root;stage=r/'.workflow-stage/test';(stage/'bin').mkdir(parents=True);(r/'bin').mkdir()
        files={}
        for name in ['horizon-engine','horizon-server']:
            (r/'bin'/name).write_bytes(b'old-'+name.encode());(stage/'bin'/name).write_bytes(b'new-'+name.encode());files['bin/'+name]=w.digest(stage/'bin'/name)
        return {'root':str(r),'stage':str(stage),'service':'test.service','apply':True,'files':files}

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
        self.assertEqual([c.args[0][3]for c in run.call_args_list if c.args[0][0]=='sudo'],['stop','start','stop','start'])

    def test_unknown_destination_never_written(self):
        p=self.payload();p['files']['../world/level.dat']='abc'
        with self.assertRaises(ValueError):remote.execute(p)


if __name__=='__main__':unittest.main()
