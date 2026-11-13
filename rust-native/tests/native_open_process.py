"""Actual offscreen native activation -> exact main hide -> entirely fake SDK."""
import copy
import json
import os
from pathlib import Path
import plistlib
import shutil
import time
import unittest
import bridge_process as native
import workspace_process as fixture
import runtime_process as rt
import actions_process as action_helpers


def command(kind='app',target='dev.fixture.inline',**fields):
    return dict(id='fixture.open',title='Native Open Fixture',kind=kind,target=target,aliases=['nativeopen'],**fields)

class NativeOpen(unittest.TestCase):
    @classmethod
    def setUpClass(cls): fixture.Workspace.setUpClass()
    def host(self,cmd=None,*,discovered=False,clipboard=None,theme=None,width=760,barrier=False,deadline_ms=None):
        h=native.Bridge(start=False);self.addCleanup(h.close)
        if deadline_ms is not None:h.env['RUYCAST_NATIVE_TEST_OPEN_DEADLINE_MS']=str(deadline_ms)
        config=copy.deepcopy(rt.DEFAULT);config['commands'] += [cmd or command()]
        if theme: config['theme']=theme
        h.env["RUYCAST_NATIVE_TEST_WIDTH"]=str(width)
        h.replace(config)
        h.env.update(DYLD_INSERT_LIBRARIES=str(fixture.SHIM),RUYCAST_WORKSPACE_FIXTURE_HOME=str(h.base),RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD='trace',RUYCAST_NATIVE_TEST_REQUIRE_HIDE='1')
        if clipboard is not None: h.env.update(RUYCAST_NATIVE_TEST_CLIPBOARD='named',RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT=clipboard)
        if discovered:
            p=h.applications/'Fixture.app/Contents';p.mkdir(parents=True)
            (p/'Info.plist').write_bytes(plistlib.dumps(dict(CFBundleIdentifier='dev.fixture.discovered',CFBundleName='Discovered Native Fixture')))
        if barrier: h.env["RUYCAST_NATIVE_TEST_TEMPLATE_BARRIER"]="1"
        h.start();h.settled()
        for key in ['DYLD_INSERT_LIBRARIES','RUYCAST_WORKSPACE_FIXTURE_HOME']: h.env.pop(key,None)
        if discovered:h.wait(lambda s:not s['appDiscoveryPending'])
        return h
    def show(self,h,query='nativeopen'):
        self.assertTrue(h.rpc(dict(op='show',query=query))['ok']);h.settled(lambda s:s['nativeUI']['visible'])
    def activate(self,h): h.probe('key-event',code=36,chars='\r',flags=0)
    def trace(self,h):
        p=h.base/'workspace-trace.jsonl';return [json.loads(line) for line in p.read_text().splitlines()] if p.exists() else []
    def opens(self,h):return [r for r in self.trace(h) if r['phase'].startswith('open-')]
    def hides(self,h):
        p=h.profile/'native-hide.jsonl';return [json.loads(line) for line in p.read_text().splitlines()] if p.exists() else []
    def wait_file(self,p):
        end=time.monotonic()+4
        while not p.exists():
            self.assertLess(time.monotonic(),end,'Own fixture did not enter '+p.name);time.sleep(.01)
    def done(self,h): return h.wait(lambda s:s['openRequestsRunning']==0 and not s['nativeUI'].get('executionRunning',False))
    def capture(self,h,name):
        view=h.probe('screenshot')
        if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(directory).parent/'native-open';out.mkdir(parents=True,exist_ok=True)
            shutil.copy2(h.profile/'native-probe.png',out/(name+'.png'))
            (out/(name+'.json')).write_text(json.dumps(view,ensure_ascii=False,indent=2))
        return view

    def test_new_show_and_query_revoke_blocked_old_lookup(self):
        for action in ['show','query']:
            h=self.host(command(target='dev.fixture.hold-resolve'))
            self.show(h);self.activate(h);self.wait_file(h.base/'resolve-entered')
            if action=='show': self.show(h,'nativeopen')
            else: h.probe('query',query='new-intent')
            (h.base/'workspace-release').write_text('release');self.done(h)
            self.assertEqual(self.opens(h),[]);self.assertEqual(self.hides(h),[])
            self.assertNotIn('fixture.open',h.cli('usage')['result'])
    def test_gui_workspace_ignores_script_timeout_and_finishes_only_on_sdk_result(self):
        for kind,target in [('url','ruycast-fixture://hold-sync'),('app','dev.fixture.hold')]:
            h=self.host(command(kind,target,timeout=.1));self.show(h);self.activate(h);self.wait_file(h.base/'open-started')
            # Cross the actual old minimum1s GUI timer while the fake SDK is held.
            time.sleep(1.2);status=h.rpc(dict(op='status'))['result'];self.assertEqual(status['openRequestsRunning'],1);self.assertEqual(status['scriptsRunning'],0);self.assertFalse(status['nativeUI']['visible'])
            self.assertNotIn('fixture.open',h.cli('usage')['result']);self.assertEqual(len(self.hides(h)),1)
            if kind=='app':h.probe('workspace-release-callbacks')
            else:(h.base/'workspace-release').write_text('release')
            self.done(h);self.assertEqual(h.cli('usage')['result']['fixture.open'],1);self.assertEqual(len(self.opens(h)),1)
    def test_gui_without_wall_timer_still_cancels_on_new_show(self):
        h=self.host(command('url','ruycast-fixture://hold-sync',timeout=.1));self.show(h);self.activate(h);self.wait_file(h.base/'open-started');time.sleep(1.2)
        self.assertEqual(h.rpc(dict(op='status'))['result']['openRequestsRunning'],1);self.show(h);self.done(h)
        (h.base/'workspace-release').write_text('release');h.wait(lambda s:s['liveExecutionWorkers']==0);self.assertTrue(h.rpc(dict(op='status'))['result']['nativeUI']['visible']);self.assertNotIn('Check its outcome','\n'.join(h.probe('read')['screenText']))
    def test_expiry_cancel_and_config_change_before_hide_never_open(self):
        for reason in ['expiry','cancel','config']:
            h=self.host(command(target='dev.fixture.hold-resolve',timeout=1),deadline_ms=250 if reason=='expiry' else None)
            self.show(h);self.activate(h);self.wait_file(h.base/'resolve-entered')
            if reason=='cancel':h.probe('key-event',code=53,chars='',flags=0)
            elif reason=='config':
                config=json.loads(h.config.read_text());config['commands'][-1]['target']='dev.fixture.changed';h.replace(config);h.cli('reload')
            self.done(h);(h.base/'workspace-release').write_text('release');h.wait(lambda s:s['liveExecutionWorkers']==0)
            self.assertEqual(self.opens(h),[]);self.assertEqual(self.hides(h),[])
    def test_failed_callback_and_post_sdk_timeout_surface_the_exact_owned_error(self):
        for target in ['dev.fixture.error','ruycast-fixture://hold-sync']:
            kind='app' if target.startswith('dev.fixture.') else 'url'
            h=self.host(command(kind,target,timeout=1),deadline_ms=250 if kind=='url' else None)
            self.show(h);self.activate(h);self.wait_file(h.base/'open-started');self.done(h)
            self.assertTrue(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])
            h.settled();view=self.capture(h,'timeout' if kind=='url' else 'callback-error');text='\n'.join(view['screenText'])
            self.assertTrue('Fixture native error' in text or 'Check its outcome' in text,text)
            self.assertEqual(len(self.hides(h)),1)
            self.assertFalse(view['windowShown'])
            if kind=='url':(h.base/'workspace-release').write_text('release')
    def test_post_effect_storage_failure_and_ranking_warning_are_visible(self):
        for file in ['usage.json','search-ranking.json']:
            h=self.host(command('url','ruycast-fixture://hold-sync',timeout=3),width=600)
            self.show(h);self.activate(h);self.wait_file(h.base/'open-started')
            self.assertEqual(h.rpc(dict(op='status'))['result']['openRequestsRunning'],1)
            blocked=h.profile/file
            if blocked.exists():blocked.unlink()
            blocked.mkdir();(h.base/'workspace-release').write_text('release');self.done(h);h.settled()
            self.assertTrue(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])
            text='\n'.join(self.capture(h,file+'-warning')['screenText'])
            self.assertTrue('usage could not be saved' in text or 'Opened, but recent choices could not be saved' in text,text)
            self.assertEqual(len(self.hides(h)),1)

    def test_new_show_after_sdk_entry_cannot_be_hidden_or_receive_old_failure(self):
        h=self.host(command('url','ruycast-fixture://hold-sync',timeout=1))
        self.show(h);self.activate(h);self.wait_file(h.base/'open-started');self.assertEqual(h.rpc(dict(op='status'))['result']['openRequestsRunning'],1);self.show(h)
        self.done(h);(h.base/'workspace-release').write_text('release');h.wait(lambda s:s['liveExecutionWorkers']==0)
        self.assertTrue(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])
        self.assertEqual(len(self.hides(h)),1)
        self.assertNotIn('Check its outcome','\n'.join(h.probe('read')['screenText']))
    def test_missing_application_and_template_validation_keep_action_uncommitted(self):
        for theme in ['light','dark']:
            h=self.host(command(target='dev.fixture.missing'),theme=theme)
            self.show(h);self.activate(h);self.done(h)
            view=h.probe('screenshot');self.assertIn('App is not installed:','\n'.join(view['screenText']));self.assertEqual(self.hides(h),[]);self.assertEqual(self.opens(h),[])
            if directory:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                out=Path(directory).parent/'native-open';out.mkdir(parents=True,exist_ok=True)
                shutil.copy2(h.profile/'native-probe.png',out/(theme+'.png'))
                (out/(theme+'.json')).write_text(json.dumps(view,ensure_ascii=False,indent=2))

if __name__=='__main__':unittest.main(verbosity=2)
