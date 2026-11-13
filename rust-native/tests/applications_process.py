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
    def test_native_palette_has_apps_but_settings_contains_only_saved_commands(self):
        host=self.host(native=True);self.bundle(host.applications,'App','dev.fixture.palette',CFBundleName='Fixture Palette');host.start();self.ready(host)
        for theme in ['light','dark']:
            cfg=json.loads(host.config.read_text());cfg['theme']=theme;host.config.write_text(json.dumps(cfg));host.rpc({'op':'reload'});host.rpc(dict(op='show',query='Fixture Palette'));host.settled()
            v=self.capture(host,'apps-'+theme);self.assertIn('Fixture Palette',v['screenText']);self.assertEqual(host.rpc({'op':'status'})['result']['nativeUI']['selectedId'],'discovered.dev.fixture.palette')
        host.rpc(dict(op='show',mode='settings',query='Fixture Palette'));host.wait(lambda s:s['managementUI']['revision']==s['managementUI']['appliedRevision'])
        v=host.rpc(dict(op='native.test',surface='settings',action='read'))['result'];self.assertEqual(v['rows'],0);self.assertIn('No commands found',v['screenText'])
        self.assertEqual(host.rpc({'op':'usage'})['result'],{});self.assertEqual(host.rpc({'op':'logs'})['result'],[])
    def test_late_discovery_preserves_real_input_text_token_and_undo(self):
        host=self.host(native=True,barrier=True);self.bundle(host.applications,'App','dev.fixture.late',CFBundleName='Fixture Late')
        cfg=copy.deepcopy(rt.DEFAULT);cfg['commands'].append(dict(id='script.fixture',title='Input Fixture',kind='script',target='/usr/bin/true',aliases=['runfixture'],inputPrompt='Message'));host.config.write_text(json.dumps(cfg));host.start();self.file(host.base/'app-discovery-ready')
        helper=inputs.NativeInput();before=helper.open(host);token=before['editorToken'];helper.type(host,'PRIVATE own input');self.assertTrue(host.probe('run-input-read')['canUndo'])
        (host.base/'app-discovery-release').write_text('go');self.ready(host);host.settled();after=host.probe('read');self.assertEqual(after['editorToken'],token);self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE own input');self.assertTrue(host.probe('run-input-read')['canUndo']);helper.type(host,' retained');self.assertEqual(host.probe('run-input-read')['text'],'PRIVATE own input retained');self.assertEqual(host.rpc({'op':'usage'})['result'],{})
    def test_shutdown_cancels_positive_waiting_discovery_without_release(self):
        host=self.host(barrier=True);host.start();self.file(host.base/'app-discovery-ready');self.assertTrue(host.rpc({'op':'status'})['result']['appDiscoveryPending']);self.assertTrue(host.rpc({'op':'quit'})['ok']);out,err=host.process.communicate(timeout=3);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());self.assertFalse((host.profile/'control.sock').exists())
    def test_fixture_refuses_escape_roots_and_missing_marker(self):
        for mode in ['escape','marker','symlink']:
            with self.subTest(mode=mode):
                host=self.host()
                if mode=='escape':host.env['RUYCAST_NATIVE_TEST_APP_ROOTS']=json.dumps(['/Applications'])
                elif mode=='marker':(host.base/'.ruycast-test-fixture').unlink()
                else:
                    link=host.base/'Escape';link.symlink_to('/Applications',target_is_directory=True);host.env['RUYCAST_NATIVE_TEST_APP_ROOTS']=json.dumps([str(link)])
                p=subprocess.run([str(HEADLESS),'--headless-test'],env=host.env,capture_output=True,text=True,timeout=4);self.assertNotEqual(p.returncode,0);self.assertIn('discovery',p.stdout.lower());self.assertFalse((host.profile/'control.sock').exists())
    def test_normal_host_refuses_headless_fixture_flag(self):
        host=self.host();normal=rt.ROOT/'target/debug/ruycast-native';p=subprocess.run([str(normal),'--headless-test'],env=host.env,capture_output=True,text=True,timeout=4);self.assertNotEqual(p.returncode,0);self.assertFalse((host.profile/'control.sock').exists())

    def test_fixture_rejects_metadata_and_localization_symlink_escape(self):
        for mode in ['contents','plist','resources','strings','hardlink']:
            with self.subTest(mode=mode):
                host=self.host();external=self.host();outside=self.bundle(external.applications,'External','dev.fixture.external',CFBundleName='Fixture External')
                app=host.applications/'Inside.app';app.mkdir()
                if mode=='contents':(app/'Contents').symlink_to(outside/'Contents',target_is_directory=True)
                else:
                    (app/'Contents').mkdir();info=outside/'Contents/Info.plist'
                    if mode=='plist':(app/'Contents/Info.plist').symlink_to(info)
                    elif mode=='hardlink':os.link(info,app/'Contents/Info.plist')
                    else:
                        (app/'Contents/Info.plist').write_bytes(info.read_bytes());resources=outside/'Contents/Resources/en.lproj';resources.mkdir(parents=True);(resources/'InfoPlist.strings').write_text('"CFBundleDisplayName" = "Fixture localized external";')
                        if mode=='resources':(app/'Contents/Resources').symlink_to(resources.parent,target_is_directory=True)
                        else:
                            local=app/'Contents/Resources/en.lproj';local.mkdir(parents=True);(local/'InfoPlist.strings').symlink_to(resources/'InfoPlist.strings')
                host.start();self.assertEqual(self.ready(host)['discoveredApps'],0);self.assertEqual(self.search(host),[])

if __name__=='__main__':unittest.main(verbosity=2)
