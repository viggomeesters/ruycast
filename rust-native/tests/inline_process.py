"""Native Settings bindings use stable editors and a single worker-owned draft."""
import copy
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
        cls.temp=tempfile.TemporaryDirectory(prefix='inline-shim-',dir=library.bridge.rt.ROOT/'target');cls.shim=Path(cls.temp.name)/'delay.dylib';subprocess.run(['clang','-dynamiclib','-Wall','-Wextra',str(library.bridge.rt.ROOT/'tests/fcntl_sync_shim.c'),'-o',str(cls.shim)],check=True,capture_output=True)
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
    def test_alias_enter_and_unchanged_focus_loss_do_not_leak_form_or_rewrite(self):
        host=self.host();self.show(host);v=self.begin(host);self.assertFalse(v['formVisible']);self.assertTrue(host.probe('inline-control-read')['hitEditor']);self.assertTrue(host.probe('inline-control-read')['underlyingHidden']);self.assertEqual(host.probe('read')['accessibilityText'].count('Aliases for Finder'),1);host.probe('alias-type',text='inlinefinder');self.capture(host,'inline-alias');self.key(host,36);self.done(host);self.assertEqual(self.command(host)['aliases'],['inlinefinder'])
        self.begin(host);before=host.config.read_bytes();host.probe('inline-focus-search');self.done(host);self.assertEqual(host.config.read_bytes(),before)
    def test_alias_focus_loss_commits_and_error_keeps_repairable_draft(self):
        host=self.host();self.show(host);self.begin(host);host.probe('alias-type',text='focusfinder');host.probe('inline-focus-search');self.done(host);self.assertEqual(self.command(host)['aliases'],['focusfinder'])
        self.begin(host);before=host.config.read_bytes();host.probe('alias-type',text='safari');host.probe('inline-focus-search');self.view(host,lambda v:any('duplicate' in s.lower() for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);host.probe('alias-type',text='repaired');self.key(host,36);self.done(host);self.assertEqual(self.command(host)['aliases'],['repaired'])
    def test_switch_commits_a_before_opening_b_and_validation_failure_keeps_a(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='savedfirst');self.begin(host,'app.safari');self.assertEqual(self.command(host)['aliases'],['savedfirst']);self.assertEqual(host.probe('read')['aliasValue'],'safari')
        host.probe('alias-type',text='savedfirst');token=host.probe('read')['editorToken'];before=host.config.read_bytes();host.probe('inline-alias',id='app.finder');self.view(host,lambda v:any('duplicate' in s.lower() for s in v['screenText']));self.assertEqual(host.probe('read')['editorToken'],token);self.assertEqual(host.config.read_bytes(),before);host.probe('alias-type',text='newsafari');self.begin(host,'app.finder');self.assertEqual(self.command(host,'app.safari')['aliases'],['newsafari'])
    def test_checkbox_transition_commits_alias_or_retains_error(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='beforetoggle');host.probe('library-enable',id='app.safari');self.done(host);self.assertEqual(self.command(host)['aliases'],['beforetoggle']);self.assertFalse(self.command(host,'app.safari')['enabled'])
        self.begin(host);host.probe('alias-type',text='safari');before=host.config.read_bytes();host.probe('library-enable',id='app.safari');self.view(host,lambda v:any('duplicate' in s.lower() for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertTrue(host.probe('read')['inlineVisible'])
    def test_inline_hotkey_records_immediately_clears_and_escape_does_not_write(self):
        host=self.host();self.show(host);self.begin(host,hotkey=True);before=host.config.read_bytes();self.key(host,53);self.done(host);self.assertEqual(host.config.read_bytes(),before)
        self.begin(host,hotkey=True);self.capture(host,'inline-hotkey');self.key(host,13,actions.CMD,'w');self.done(host);self.assertEqual(self.command(host)['hotkey'],'cmd+w');self.assertTrue(host.rpc({'op':'status'})['result']['managementUI']['visible']);self.begin(host,hotkey=True);self.key(host,51);self.done(host);self.assertNotIn('hotkey',self.command(host))
    def test_reload_preserves_actual_typing_and_composition(self):
        host=self.host();self.show(host);self.begin(host);host.probe('alias-type',text='mydraft');token=host.probe('read')['editorToken'];config=json.loads(host.config.read_bytes());config['clipboard']['limit']=84;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled();self.assertEqual(host.probe('read')['editorToken'],token);host.probe('alias-type',text='x');self.assertIn('x',host.probe('read')['aliasValue']);self.assertTrue(host.probe('inline-mark',text='日本')['marked']);config['commands'][0]['title']='Finder renamed';host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled();self.assertTrue(host.probe('inline-marked-read')['marked']);host.probe('inline-unmark');self.key(host,53)
    def test_removed_anchor_falls_back_without_losing_marked_text_or_persisting(self):
        host=self.host();self.show(host);self.begin(host);token=host.probe('read')['editorToken'];self.assertTrue(host.probe('inline-mark',text='日本')['marked']);config=json.loads(host.config.read_bytes());config['commands']=[c for c in config['commands'] if c['id']!='app.finder'];host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled();before=host.config.read_bytes();v=host.probe('inline-marked-read');self.assertTrue(v['marked']);self.assertTrue(v['formVisible']);self.assertTrue(host.probe('inline-control-read')['hitEditor']);self.assertEqual(v['editorToken'],token);self.assertEqual(host.config.read_bytes(),before);self.capture(host,'inline-anchor-fallback');host.probe('inline-unmark');self.key(host,53)
    def test_scroll_far_unselected_binding_selects_target_and_preserves_composition(self):
        host=self.host();commands=[dict(id=f'url.{i:03}',title=f'Item {i:03}',kind='url',target=f'https://example.org/{i}',aliases=[f'alias{i}']) for i in range(200)];self.fixture(host,commands);self.show(host,'');self.begin(host,'url.150');host.settled(lambda s:s['managementUI']['selectedId']=='url.150');self.assertTrue(host.probe('inline-mark',text='日本')['marked']);host.probe('inline-scroll',y=0.);self.assertTrue(host.probe('inline-marked-read')['marked']);host.probe('inline-scroll',y=150*39.);self.assertTrue(host.probe('inline-marked-read')['marked']);self.capture(host,'inline-scrolled');host.probe('inline-unmark');self.key(host,53)
    def test_marked_alias_refuses_binding_and_checkbox_transition(self):
        host=self.host();self.show(host,'');self.begin(host);token=host.probe('read')['editorToken'];self.assertTrue(host.probe('inline-mark',text='日本')['marked']);before=host.config.read_bytes();host.probe('library-enable',id='app.safari');host.probe('inline-alias',id='app.safari');host.settled();self.assertEqual(host.probe('read')['editorToken'],token);self.assertEqual(host.config.read_bytes(),before);self.assertTrue(host.probe('inline-marked-read')['marked']);host.probe('inline-unmark');self.key(host,53)
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
    def test_worker_rejection_acknowledges_and_keeps_the_native_draft_typable(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='retained');token=host.probe('read')['editorToken'];before=host.config.read_bytes();host.probe('inline-stale-switch');self.view(host,lambda v:any('Selection changed' in s for s in v['screenText']));self.assertEqual(host.probe('read')['editorToken'],token);host.probe('alias-type',text='x');self.assertIn('x',host.probe('read')['aliasValue']);self.assertEqual(host.config.read_bytes(),before)
    def test_cancelled_parent_filters_an_already_published_next_editor(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='published');self.begin(host,'app.safari');host.probe('alias-type',text='mustnotpublish');before=host.config.read_bytes();host.probe('cancel-inline-origin');self.done(host);host.probe('save-editor');host.settled();self.assertEqual(host.config.read_bytes(),before)
    def test_navigation_commits_before_leaving_and_validation_error_keeps_context(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='beforenav');host.probe('library-tab',tab='snippets');self.done(host);self.assertEqual(self.command(host)['aliases'],['beforenav']);self.assertEqual(host.probe('library-read')['tab'],'snippets');host.probe('library-tab',tab='commands');host.settled();self.begin(host);host.probe('alias-type',text='safari');before=host.config.read_bytes();host.probe('library-tab',tab='quicklinks');self.view(host,lambda v:any('duplicate' in s.lower() for s in v['screenText']));self.assertEqual(host.probe('library-read')['tab'],'commands');self.assertEqual(host.config.read_bytes(),before);host.probe('alias-type',text='repairednav');host.probe('library-filter',filter='disabled');self.done(host);self.assertEqual(self.command(host)['aliases'],['repairednav']);self.assertEqual(host.probe('library-read')['filter'],'disabled')
    def test_add_and_title_edit_commit_the_previous_inline_draft(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='beforecreate');host.probe('library-add',kind='snippet');self.view(host,lambda v:v['formVisible'] and 'New Snippet' in v['screenText']);self.assertEqual(self.command(host)['aliases'],['beforecreate']);host.probe('cancel-editor');self.done(host);self.begin(host);host.probe('alias-type',text='beforeedit');rows=host.probe('library-read')['rows'];index=next(i for i,r in enumerate(rows) if r['id']=='app.safari');host.probe('click',row=index);self.view(host,lambda v:v['formVisible'] and 'Name' in v['screenText']);self.assertEqual(self.command(host)['aliases'],['beforeedit']);self.assertEqual(host.probe('command-read')['value']['id'],'app.safari')
    def test_native_tracking_defers_autosave_until_binding_action_has_run(self):
        host=self.host();self.show(host,'');self.begin(host);host.probe('alias-type',text='beforetracking');host.probe('inline-tracking-switch',id='app.safari');self.view(host,lambda v:v['inlineVisible'] and v['aliasValue']=='safari');self.assertEqual(self.command(host)['aliases'],['beforetracking']);host.probe('alias-type',text='safaridraft');self.assertTrue(host.probe('inline-control-read')['hitEditor'])
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
