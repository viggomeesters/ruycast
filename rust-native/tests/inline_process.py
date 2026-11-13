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
    def test_repeated_focus_return_coalesces_one_autosave_and_stays_typable(self):
        host=self.host();self.show(host);self.begin(host);host.probe('alias-type',text='coalesced');before=host.config.read_bytes();host.probe('inline-repeat-focus');host.settled();control=host.probe('inline-control-read');self.assertEqual(control['autosaveSlots'],1);self.assertTrue(control['hitEditor']);self.assertEqual(host.config.read_bytes(),before);host.probe('alias-type',text='x');host.probe('inline-focus-search');self.done(host);self.assertIn('x',self.command(host)['aliases'][0])
    def test_group_collapse_commits_the_inline_alias_and_search_still_finds_it(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='groupfinder');host.probe('click',row=0);self.done(host);self.assertEqual(self.command(host)['aliases'],['groupfinder']);self.assertFalse(any(r['id']=='app.finder' for r in host.probe('library-read')['rows']));host.probe('query',query='groupfinder');host.settled(lambda s:s['managementUI']['selectedId']=='app.finder')
    def test_query_typing_saves_alias_without_clobbering_query_and_retains_errors(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='beforequery');host.probe('inline-focus-and-query',query='safari');self.done(host);self.assertEqual(self.command(host)['aliases'],['beforequery']);self.assertEqual(host.probe('read')['input'],'safari');self.assertEqual(host.probe('read')['modelQuery'],'safari')
        self.show(host,'');self.begin(host);host.probe('alias-type',text='safari');before=host.config.read_bytes();host.probe('inline-focus-and-query',query='safari');self.view(host,lambda v:v['formVisible'] and any('duplicate' in s.lower() for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('read')['input'],'safari');host.probe('alias-type',text='queryrepair');self.key(host,36);self.done(host);self.assertEqual(self.command(host)['aliases'],['queryrepair']);self.assertEqual(host.probe('read')['modelQuery'],'safari')
    def test_alias_undo_history_is_local_to_the_current_bound_draft(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='firstprivate');self.assertTrue(host.probe('alias-undo-state')['canUndo']);self.begin(host,'app.safari');self.assertFalse(host.probe('alias-undo-state')['canUndo']);self.assertEqual(host.probe('read')['aliasValue'],'safari');host.probe('alias-type',text='secondprivate');self.key(host,6,actions.CMD,'z');self.assertEqual(host.probe('read')['aliasValue'],'safari')
    def test_close_and_app_resign_never_autosave_dirty_alias(self):
        host=self.host();self.show(host);self.begin(host);host.probe('alias-type',text='discard');before=host.config.read_bytes();host.probe('window-close');host.settled(lambda s:not s['managementUI']['visible']);self.assertEqual(host.config.read_bytes(),before);self.show(host);self.begin(host);host.probe('alias-type',text='hold');host.probe('resign-app');host.probe('inline-focus-search');host.settled();self.assertEqual(host.config.read_bytes(),before);self.assertTrue(host.rpc({'op':'status'})['result']['managementUI']['editorOpen'])

if __name__=='__main__':unittest.main(verbosity=2)
