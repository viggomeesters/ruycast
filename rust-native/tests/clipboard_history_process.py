#!/usr/bin/env python3
"""Original clipboard routes through actual Rust owner/main named-board fixtures.
No general board, screen capture, app activation or physical input is permitted.
"""
import copy,json,os,shutil,subprocess,time,unittest,zlib,struct,hashlib
from pathlib import Path
import runtime_process as rt
import bridge_process as native
ROOT=rt.ROOT
native.TOOLS=Path(os.environ.get('RUYCAST_TEST_APP_HOST',ROOT/'target/debug/ruycast-native'))
rt.HOST=native.TOOLS
rt.CLI=Path(os.environ.get('RUYCAST_TEST_CLI',ROOT.parent/'rust/target/debug/ruycast'))
SHIM=ROOT/'target/clipboard-history-effects.dylib'
OUT=ROOT/'target/clipboard-evidence'
def png():
    chunk=lambda t,b:struct.pack('>I',len(b))+t+b+struct.pack('>I',zlib.crc32(t+b))
    return b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',2,2,8,6,0,0,0))+chunk(b'IDAT',zlib.compress((b'\0'+bytes([128,64,32,255])*2)*2))+chunk(b'IEND',b'')
class ClipboardHistory(unittest.TestCase):
 @classmethod
 def setUpClass(cls):
    OUT.mkdir(parents=True,exist_ok=True)
    target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT.parent/'.build/full-rust-release/native'))
    artifact=Path(os.environ.get('RUYCAST_TEST_HISTORY_SDK',target/'release/examples/libclipboard_history_fixture.dylib'))
    if not artifact.is_absolute() or not artifact.is_file():
        raise RuntimeError('Build --features test-tools --example clipboard-history-fixture first')
    shutil.copyfile(artifact,SHIM)
    if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
        out=Path(evidence);out.mkdir(parents=True,exist_ok=True)
        sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
        sources=[ROOT/'tests/fixtures'/name for name in ['workspace_sdk.rs','snippet_effects_sdk.rs','clipboard_history_sdk.rs','clipboard_history_fixture.rs']]
        sources.extend([ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT.parent/'rust/Cargo.toml',ROOT.parent/'rust-toolchain.toml'])
        (out/'clipboard-history-bindings.json').write_text(json.dumps(dict(sources={str(p.relative_to(ROOT.parent)):sha(p) for p in sources},artifacts={str(SHIM.relative_to(ROOT.parent)):sha(SHIM)}),indent=2))
 def host(self,*,width=760,theme='light',config=None):
    h=native.Bridge(start=False);self.addCleanup(h.close)
    cfg=copy.deepcopy(rt.DEFAULT);cfg['theme']=theme
    if config:cfg['clipboard'].update(config)
    h.config.write_text(json.dumps(cfg))
    h.env.update(DYLD_INSERT_LIBRARIES=str(SHIM),RUYCAST_WORKSPACE_FIXTURE_HOME=str(h.base),RUYCAST_NATIVE_TEST_CLIPBOARD='named',RUYCAST_NATIVE_TEST_SNIPPET='1',RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD='1',RUYCAST_NATIVE_TEST_WIDTH=str(width))
    h.start();h.settled();return h
 def fixture(self,h,payload,**fields):
    value=h.probe('clipboard-fixture',payload=payload,defer=True,**fields)
    time.sleep(.5)
    self.assertTrue(value['namedBoard']);return value
 def items(self,h):return h.rpc(dict(op='clipboard.list'))['result']
 def wait_items(self,h,n):h.wait(lambda s:s['clipboardCount']==n)
 def capture(self,h,name):
    h.settled();value=h.probe('screenshot');self.assertFalse(value['windowShown']);self.assertFalse(value['windowKey']);self.assertFalse(value['windowMain'])
    shutil.copy2(h.profile/'native-probe.png',OUT/(name+'.png'));(OUT/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2));return value
 def show(self,h):self.assertTrue(h.rpc(dict(op='show',mode='clipboard'))['ok']);h.settled(lambda s:s['nativeUI']['visible'])
 def test_cli_import_export_search_copy_paste_delete_clear(self):
    h=self.host();values=[dict(id='raycast-clip',text='Synthetic CLI Å',date='2026-11-01T08:00:00Z',source='fixture')]
    values[0]['date']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
    file=h.base/'import.json';file.write_text(json.dumps(values));self.assertEqual(h.cli('clipboard','import','@'+str(file))['result']['accepted'],1)
    self.assertEqual(h.cli('clipboard','list','cli å')['result'][0]['id'],'raycast-clip');self.assertEqual(h.cli('clipboard','export')['result'],h.cli('clipboard','list')['result'])
    self.assertEqual(h.cli('clipboard','copy','raycast-clip')['result']['state'],'copied');time.sleep(.5);self.assertEqual([c['id'] for c in self.items(h)],['raycast-clip'])
    self.show(h);result=h.cli('clipboard','paste','raycast-clip');self.assertEqual(result['result']['state'],'copied');h.wait(lambda s:s['snippetRequestsRunning']==0)
    trace=(h.base/'workspace-trace.jsonl').read_text();self.assertIn('snippet-post',trace);self.assertNotIn('unexpected-general',trace)
    self.assertEqual(h.cli('clipboard','delete','raycast-clip')['result']['count'],0);self.assertEqual(h.cli('clipboard','clear')['result']['count'],0)
 def test_native_text_image_empty_error_busy_light_dark_narrow_wide(self):
    for width,theme in [(600,'light'),(760,'dark')]:
        h=self.host(width=width,theme=theme);self.show(h);self.capture(h,f'empty-{width}-{theme}')
        self.fixture(h,{'Text':'Synthetic clipboard text\nOne deliberate second line.'});self.wait_items(h,1);text=self.capture(h,f'text-{width}-{theme}');self.assertIn('Synthetic clipboard text\nOne deliberate second line.',text['screenText']);self.assertNotIn('Clipboard is empty',text['screenText']);self.assertIn('Search clipboard',text['screenText'])
        (h.profile/'clipboard-preview-hold').write_text('own fixture')
        self.fixture(h,{'Image':list(png())});self.wait_items(h,2);h.probe('query',query='Image');h.settled()
        end=time.monotonic()+3
        while not (h.profile/'clipboard-preview-entered').exists() and time.monotonic()<end:time.sleep(.01)
        self.assertTrue((h.profile/'clipboard-preview-entered').exists());busy=self.capture(h,f'busy-{width}-{theme}');self.assertIn('Loading preview…',busy['screenText'])
        (h.profile/'clipboard-preview-hold').unlink();time.sleep(.15);self.capture(h,f'image-{width}-{theme}')
        image=self.items(h)[0];(h.profile/image['dataFile']).unlink()
        h.probe('query',query='no-match');h.settled();h.probe('query',query='Image');h.settled();time.sleep(.15);error=self.capture(h,f'error-{width}-{theme}');self.assertTrue(any('unavailable' in t.lower() for t in error['screenText']))
 def test_native_preferences_save_is_revision_guarded_and_validated(self):
    h=self.host();self.assertTrue(h.rpc(dict(op='show',mode='settings'))['ok'])
    h.wait(lambda s:s['managementUI']['visible'] and s['managementUI']['revision']==s['managementUI']['appliedRevision'])
    def probe(action,**fields):
        response=h.rpc(dict(op='native.test',surface='settings',action=action,**fields));self.assertTrue(response['ok'],response);return response['result']
    probe('clipboard-preferences');h.wait(lambda s:s['managementUI']['tab']=='clipboard');time.sleep(.05)
    view=probe('screenshot');shutil.copy2(h.profile/'native-settings-probe.png',OUT/'preferences-light.png');(OUT/'preferences-light.json').write_text(json.dumps(view,indent=2))
    before=json.loads(h.config.read_text());probe('clipboard-preferences-save',limit=0);time.sleep(.15);self.assertEqual(json.loads(h.config.read_text()),before)
    probe('clipboard-preferences-save',limit=12,retentionDays=7,excludedApps=['dev.fixture.private']);h.wait(lambda s:s['configurationRevision']>0 and not s['configurationUpdating'])
    actual=json.loads(h.config.read_text())['clipboard'];self.assertEqual(actual['limit'],12);self.assertEqual(actual['retentionDays'],7);self.assertEqual(actual['excludedApps'],['dev.fixture.private'])
if __name__=='__main__':unittest.main(verbosity=2)
