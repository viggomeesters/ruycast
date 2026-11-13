"""Actual worker/FileManager/catalog with only own synthetic application bundles."""
import copy
import json
import os
from pathlib import Path
import plistlib
import subprocess
import time
import unittest
import runtime_process as rt
import bridge_process as bridge
import input_process as inputs

HEADLESS=rt.ROOT/'target/headless-tools/debug/ruycast-native'
class Applications(unittest.TestCase):
    def host(self,native=False,barrier=False):
        rt.HOST=HEADLESS
        host=bridge.Bridge(start=False) if native else rt.Host(start=False)
        self.addCleanup(host.close)
        if barrier:host.env['RUYCAST_NATIVE_TEST_APP_BARRIER']='1'
        return host
    def bundle(self,root,name,identifier,**info):
        path=root/(name+'.app');(path/'Contents').mkdir(parents=True)
        value=dict(CFBundleIdentifier=identifier,CFBundlePackageType='APPL',**info)
        (path/'Contents/Info.plist').write_bytes(plistlib.dumps(value));return path
    def ready(self,host):
        return host.wait(lambda s:not s['appDiscoveryPending'])
    def file(self,path):
        end=time.monotonic()+3
        while time.monotonic()<end:
            if path.exists():return path.read_text()
            time.sleep(.01)
        self.fail('Missing own positive barrier '+path.name)
    def search(self,host,query='Fixture'):
        return host.rpc(dict(op='search',query=query,limit=0))['result']
    def capture(self,host,name):
        value=host.probe('screenshot');self.assertFalse(value['windowShown'] or value['windowKey'] or value['windowMain'])
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            target=Path(out).parent/'applications';target.mkdir(exist_ok=True)
            (target/(name+'.png')).write_bytes((host.profile/'native-probe.png').read_bytes())
            (target/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value
    def test_late_discovery_preserves_real_input_text_token_and_undo(self):
        host=self.host(native=True,barrier=True);self.bundle(host.applications,'App','dev.fixture.late',CFBundleName='Fixture Late')
        cfg=copy.deepcopy(rt.DEFAULT);cfg['commands'].append(dict(id='script.fixture',title='Input Fixture',kind='script',target='/usr/bin/true',aliases=['runfixture'],inputPrompt='Message'));host.config.write_text(json.dumps(cfg));host.start();self.file(host.base/'app-discovery-ready')
        helper=inputs.NativeInput();before=helper.open(host);token=before['editorToken'];helper.type(host,'PRIVATE own input');self.assertTrue(host.probe('run-input-read')['canUndo'])
        (host.base/'app-discovery-release').write_text('go');self.ready(host);host.settled();after=host.probe('read');self.assertEqual(after['editorToken'],token);self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE own input');self.assertTrue(host.probe('run-input-read')['canUndo']);helper.type(host,' retained');self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE own input retained');self.assertEqual(host.rpc({'op':'usage'})['result'],{})
    def test_normal_host_refuses_headless_fixture_flag(self):
        host=self.host();normal=rt.ROOT/'target/debug/ruycast-native';p=subprocess.run([str(normal),'--headless-test'],env=host.env,capture_output=True,text=True,timeout=4);self.assertNotEqual(p.returncode,0);self.assertFalse((host.profile/'control.sock').exists())


if __name__=='__main__':unittest.main(verbosity=2)
