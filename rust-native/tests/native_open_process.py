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
