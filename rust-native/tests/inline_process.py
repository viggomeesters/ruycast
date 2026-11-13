"""Native Settings bindings use stable editors and a single worker-owned draft."""
import copy
import os
import json
import unittest
import tempfile
import subprocess
from pathlib import Path
import library_process as library
import actions_process as actions
from runtime_process import DEFAULT

class Inline(unittest.TestCase):
    host=library.Library.host;show=library.Library.show;view=library.Library.view;key=library.Library.key;command=library.Library.command;capture=library.Library.capture;fixture=library.Library.fixture
    def key(self,host,code,flags=0,chars='',repeated=False):
        return actions.ActionsProcesses.key(self,host,code,flags,'\r' if code in [36,76] and not chars else chars,repeated)
    @classmethod
    def setUpClass(cls):
        cls.temp=tempfile.TemporaryDirectory(prefix='inline-shim-',dir=library.bridge.rt.ROOT/'target')
        target=Path(os.environ.get('CARGO_TARGET_DIR',library.bridge.rt.ROOT.parent/'.build/full-rust-release/native'))
        cls.shim=Path(os.environ.get('RUYCAST_TEST_FCNTL_SHIM',target/'release/examples/libfcntl_sync_shim.dylib'))
        if not cls.shim.is_absolute() or not cls.shim.is_file():
            raise RuntimeError('Build --features test-tools --example fcntl-sync-shim and set RUYCAST_TEST_FCNTL_SHIM first')
    @classmethod
    def tearDownClass(cls):cls.temp.cleanup()
    def slow_host(self,held=False):
        host=library.SettingsHost(start=False);self.addCleanup(host.close);host.env.update(DYLD_INSERT_LIBRARIES=str(self.shim),RUYCAST_TEST_FSYNC_DIRECTORY=str(host.profile),RUYCAST_TEST_FSYNC_MARKER=str(host.profile/'inline-sync-entered'),RUYCAST_TEST_FSYNC_DELAY_MS='500',RUYCAST_TEST_FSYNC_DELAY_ONLY='1')
        if held:host.env['RUYCAST_TEST_FSYNC_RELEASE']=str(host.profile/'inline-sync-release')
        host.start();host.settled();self.assertFalse((host.profile/'inline-sync-entered').exists());return host
    def begin(self,host,id='app.finder',hotkey=False):
        before=host.probe('read')['editorToken'];host.probe('inline-hotkey' if hotkey else 'inline-alias',id=id)
        return self.view(host,lambda v:v['inlineVisible'] and v['editorId']==id and v['editorToken']!=before and (v['recording'] if hotkey else True))
    def done(self,host):return host.settled(lambda s:not s['managementUI']['editorOpen'])
    def cancel_during_save(self,checkbox):
        host=self.slow_host();self.show(host,'');self.begin(host);host.probe('alias-type',text='publishedbeforecancel');before=self.command(host,'app.safari');host.probe('schedule-inline-cancel');host.probe('library-enable' if checkbox else 'inline-alias',id='app.safari');self.done(host);self.assertEqual((host.profile/'inline-sync-entered').read_bytes(),b'injected');self.assertEqual(self.command(host)['aliases'],['publishedbeforecancel']);self.assertEqual(self.command(host,'app.safari'),before)
    def test_command_w_closes_settings_during_held_async_alias_save(self):
        import time
        host=self.slow_host();self.show(host,'');self.begin(host);host.probe('alias-type',text='published-before-close')
        host.probe('save-editor');marker=host.profile/'inline-sync-entered';end=time.monotonic()+3
        while not marker.exists():
            self.assertLess(time.monotonic(),end);time.sleep(.002)
        self.key(host,13,actions.CMD,'w')
        host.wait(lambda s:not s['managementUI']['visible'])
        host.wait(lambda s:not s['configurationUpdating'])
        self.assertEqual(self.command(host)['aliases'],['published-before-close'])
    def test_cancel_during_actual_directory_sync_prevents_next_binding(self):self.cancel_during_save(False)
    def test_cancel_during_actual_directory_sync_prevents_next_checkbox_write(self):self.cancel_during_save(True)
    def test_focus_end_after_interim_ack_does_not_repeat_transition_save(self):
        import time
        host=self.slow_host(held=True);self.show(host,'');self.begin(host);host.probe('alias-type',text='once-before-switch')
        host.probe('inline-alias',id='app.safari');marker=host.profile/'inline-sync-entered';end=time.monotonic()+3
        try:
            while not marker.exists():
                self.assertLess(time.monotonic(),end);time.sleep(.002)
            self.view(host,lambda v:v['editorSaving'] and not v['editorRequestPending'])
            host.probe('inline-alias',id='app.terminal');host.probe('save-editor')
            host.probe('library-enable',id='app.safari');host.probe('library-tab',tab='snippets');host.probe('library-add',kind='snippet')
            self.assertTrue(host.probe('read')['editorSaving']);host.probe('inline-focus-search')
        finally:(host.profile/'inline-sync-release').write_text('release')
        self.view(host,lambda v:v['inlineVisible'] and v['editorId']=='app.safari' and not v['editorSaving'])
        self.assertEqual(self.command(host)['aliases'],['once-before-switch']);self.assertTrue(self.command(host,'app.safari').get('enabled',True))
        self.assertEqual(host.probe('library-read')['tab'],'commands');host.probe('alias-type',text='still-typable')
        self.assertTrue(host.probe('inline-control-read')['hitEditor'])

if __name__=='__main__':unittest.main(verbosity=2)
