"""Actual private-write syscall holds/failures; fake panel before construction."""
import json,os,shutil,subprocess,time,unittest
from pathlib import Path
from library_process import SettingsHost
import runtime_process as rt
class Exports(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.shim=rt.ROOT/'target/dashboard-export-shim.dylib';subprocess.run(['clang','-Wall','-Wextra','-Werror','-dynamiclib',str(rt.ROOT/'tests/dashboard_export_shim.c'),'-o',str(cls.shim)],check=True,capture_output=True)
    def host(self):
        h=SettingsHost(start=False);self.addCleanup(h.close);h.env.update(DYLD_INSERT_LIBRARIES=str(self.shim),RUYCAST_DASHBOARD_FIXTURE_HOME=str(h.base));h.start();h.settled();h.rpc(dict(op='show',mode='analytics'));h.settled(lambda s:s['managementUI']['tab']=='dashboard');folder=h.base/'export';folder.mkdir(mode=0o755);folder.chmod(0o755);self.addCleanup(lambda:(h.base/'export-file-sync-release').touch() if h.base.exists() else None);self.addCleanup(lambda:(h.base/'export-rename-release').touch() if h.base.exists() else None);return h,folder/'diagnostics.json'
    def wait(self,h,predicate):
        end=time.monotonic()+5
        while time.monotonic()<end:
            v=h.probe('dashboard-read')
            if predicate(v):return v
            time.sleep(.01)
        self.fail(v)
    def entered(self,h,name):
        end=time.monotonic()+3
        while time.monotonic()<end:
            if (h.base/name).exists():return
            time.sleep(.01)
        self.fail('Missing actual syscall hold '+name)
    def capture(self,h,name):
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            v=h.probe('dashboard-screenshot');folder=Path(folder);folder.mkdir(parents=True,exist_ok=True);shutil.copy2(h.profile/'native-dashboard-probe.png',folder/(name+'.png'));(folder/(name+'.json')).write_text(json.dumps(v,ensure_ascii=False,indent=2))
    def begin(self,h,path):
        h.probe('dashboard-control',index=2);v=h.probe('dashboard-read');self.assertTrue(v['fixtureDialog']);self.assertTrue(h.probe('dashboard-dialog-complete',serial=v['dialogSerial'],token=v['surface']['token'],accepted=True,path=str(path))['accepted'])
    def test_positive_held_export_global_reset_order_and_parent_mode(self):
        h,path=self.host();(h.base/'hold-export-file-sync').touch();self.begin(h,path);self.entered(h,'export-file-sync-entered');self.assertTrue(h.rpc(dict(op='status'))['ok']);self.assertFalse(h.rpc(dict(op='analytics.reset',confirmed=True))['ok']);self.assertTrue(self.wait(h,lambda v:v['surface']['busy'])['surface']['busy']);self.capture(h,'export-busy');(h.base/'export-file-sync-release').touch();self.wait(h,lambda v:not v['surface']['busy'] and v['surface']['message']=='Diagnostics exported');self.assertEqual(path.parent.stat().st_mode&0o777,0o755);self.assertEqual(path.stat().st_mode&0o777,0o600);self.assertNotIn('commands',json.loads(path.read_text()));self.assertTrue(h.rpc(dict(op='analytics.reset',confirmed=True))['ok'])
    def test_file_sync_and_rename_failure_preserve_prior_destination(self):
        for fault,marker in [('fail-export-file-sync','export-file-sync-failed'),('fail-export-rename','export-rename-failed')]:
            h,path=self.host();path.write_bytes(b'prior');(h.base/fault).touch();self.begin(h,path);self.wait(h,lambda v:not v['surface']['busy'] and 'Could not export' in v['surface']['message']);self.assertTrue((h.base/marker).exists());self.assertEqual(path.read_bytes(),b'prior');self.assertEqual(path.parent.stat().st_mode&0o777,0o755)
    def test_postrename_parent_sync_failure_reports_published_outcome(self):
        h,path=self.host();path.write_bytes(b'prior');(h.base/'fail-export-parent-sync').touch();self.begin(h,path);v=self.wait(h,lambda v:not v['surface']['busy'] and 'durability could not be confirmed' in v['surface']['message']);self.assertTrue((h.base/'export-parent-sync-failed').exists());self.assertIsInstance(json.loads(path.read_text()),dict);self.assertIn('Check the file before retrying',v['surface']['message']);h.probe('dashboard-size',width=680,height=480);self.capture(h,'export-published-warning')
    def test_close_before_commit_revokes_destination_without_feedback_to_reopened_window(self):
        h,path=self.host();path.write_bytes(b'prior');(h.base/'hold-export-file-sync').touch();self.begin(h,path);self.entered(h,'export-file-sync-entered');h.probe('window-close');h.settled(lambda s:not s['managementUI']['visible']);h.rpc(dict(op='show',mode='analytics'));h.settled(lambda s:s['managementUI']['visible']);(h.base/'export-file-sync-release').touch();self.wait(h,lambda v:not v['surface']['busy']);self.assertEqual(path.read_bytes(),b'prior');self.assertEqual(h.probe('dashboard-read')['surface']['message'],'')
    def test_commit_entered_before_close_keeps_published_outcome_and_retires_feedback(self):
        h,path=self.host();path.write_bytes(b'prior');(h.base/'hold-export-rename').touch();self.begin(h,path);self.entered(h,'export-rename-entered');h.probe('window-close');h.settled(lambda s:not s['managementUI']['visible']);h.rpc(dict(op='show',mode='analytics'));h.settled(lambda s:s['managementUI']['visible']);(h.base/'export-rename-release').touch();self.wait(h,lambda v:not v['surface']['busy']);self.assertIsInstance(json.loads(path.read_text()),dict);self.assertEqual(h.probe('dashboard-read')['surface']['message'],'')
if __name__=='__main__':unittest.main(verbosity=2)
