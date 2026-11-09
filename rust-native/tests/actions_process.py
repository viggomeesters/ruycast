"""Real native Actions/binding controls against disposable Rust profiles."""
import copy
import fcntl
import json
import time
import unittest
import bridge_process as bridge
from runtime_process import DEFAULT

CMD=1<<20;OPT=1<<19;SHIFT=1<<17;CTRL=1<<18

class ActionsProcesses(unittest.TestCase):
    host=bridge.NativeBridge.host
    capture=bridge.NativeBridge.capture
    # Reuse fixture/capture helpers without inheriting the other suite's tests.
    def view(self,host,predicate=lambda v:True):
        end=time.monotonic()+4
        while time.monotonic()<end:
            value=host.probe('read')
            if predicate(value):return value
            time.sleep(.01)
        raise AssertionError('Native action/editor did not reach expected state: '+json.dumps(value))
    def show(self,host,query='finder'):
        self.assertTrue(host.rpc({'op':'show','query':query})['ok']);host.settled(lambda s:s['nativeUI']['visible']);self.view(host,lambda v:v['input']==query)
    def key(self,host,code,flags=0,chars='',repeated=False):return host.probe('key-event',code=code,flags=flags,chars=chars,repeat=repeated)
    def alias(self,host):self.key(host,43,CMD|OPT,',');return self.view(host,lambda v:v['formVisible'] and 'Alias' in v['screenText'])
    def hotkey(self,host):self.key(host,43,CMD|SHIFT,',');return self.view(host,lambda v:v['formVisible'] and v['recording'])
    def saved(self,host):return self.view(host,lambda v:not v['formVisible'])
    def command(self,host,id='app.finder'):return next(c for c in json.loads(host.config.read_bytes())['commands'] if c['id']==id)
    def action(self,host,name):
        if not host.probe('read')['actionsVisible']:self.key(host,40,CMD,'k')
        view=self.view(host,lambda v:v['actionsVisible']);index=next(i for i,label in enumerate(view['actionLabels']) if label.startswith(name));host.probe('action-button',index=index)

    def test_actions_keyboard_badges_and_escape_keeps_palette(self):
        host=self.host();self.show(host)
        self.key(host,40,CMD,'k');view=self.view(host,lambda v:v['actionsVisible'])
        self.assertTrue(any(v.startswith('Set Alias…') and '⌥' in v for v in view['actionLabels']))
        self.assertTrue(any(v.startswith('Set Hotkey…') and '⇧' in v for v in view['actionLabels']))
        self.capture(host,'actions-menu')
        self.key(host,53);self.view(host,lambda v:not v['actionsVisible']);self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        self.key(host,43,CMD|OPT,',',True);self.assertFalse(host.probe('read')['formVisible'])
        self.alias(host);self.key(host,53);self.saved(host)
        self.key(host,40,CMD,'k');self.view(host,lambda v:v['actionsVisible']);self.key(host,125);self.key(host,125);self.key(host,36)
        self.view(host,lambda v:v['formVisible'] and 'Alias' in v['screenText']);self.key(host,53);self.saved(host)
    def test_alias_save_clear_and_cli_resolution(self):
        host=self.host();self.show(host);self.alias(host)
        host.probe('alias-input',text='myfinder, büro');self.capture(host,'alias-edit')
        self.key(host,36,CMD,'\r');self.saved(host)
        self.assertEqual(self.command(host)['aliases'],['myfinder','büro'])
        self.assertEqual(host.cli('search','büro')['result'][0]['id'],'app.finder')
        self.alias(host);host.probe('alias-input',text='');host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['aliases'],[])
    def test_alias_conflict_keeps_draft_and_allows_repair(self):
        host=self.host();self.show(host);self.alias(host);before=host.config.read_bytes()
        host.probe('alias-input',text='SAFARI');host.probe('save-editor')
        view=self.view(host,lambda v:any('duplicate' in s.lower() for s in v['screenText']))
        self.assertEqual(view['aliasValue'],'SAFARI');self.assertEqual(host.config.read_bytes(),before);self.capture(host,'alias-conflict')
        host.probe('alias-type',text='x');self.assertIn('x',host.probe('read')['aliasValue'])
        host.probe('alias-input',text='repairedfinder');host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['aliases'],['repairedfinder'])
    def test_failed_async_save_restores_native_field_focus_and_undo(self):
        host=self.host();self.show(host);self.alias(host);before=host.config.read_bytes()
        host.probe('alias-input',text='');host.probe('alias-type',text='SAFARI')
        self.assertTrue(host.probe('alias-undo-state')['canUndo'])
        host.probe('save-editor')
        self.view(host,lambda v:v['saveEnabled'] and any('duplicate' in s.lower() for s in v['screenText']))
        self.assertEqual(host.config.read_bytes(),before)
        self.assertTrue(host.probe('alias-undo-state')['canUndo'])
        self.key(host,6,CMD,'z');self.assertNotEqual(host.probe('read')['aliasValue'],'SAFARI')
        self.key(host,6,CMD|SHIFT,'z');self.assertEqual(host.probe('read')['aliasValue'],'SAFARI')

    def test_other_external_fields_merge_but_same_alias_change_conflicts(self):
        host=self.host();self.show(host);self.alias(host)
        config=json.loads(host.config.read_bytes());config['clipboard']['limit']=77;config['commands'][0]['title']='Externally renamed Finder';config['commands'][1]['title']='Other title';host.replace(config)
        host.probe('alias-input',text='mine');host.probe('save-editor');self.saved(host)
        current=json.loads(host.config.read_bytes());self.assertEqual(current['clipboard']['limit'],77);self.assertEqual(current['commands'][0]['title'],'Externally renamed Finder');self.assertEqual(current['commands'][1]['title'],'Other title')
        self.alias(host);config=current;config['commands'][0]['aliases']=['outside'];host.replace(config);before=host.config.read_bytes();host.probe('alias-input',text='draft');host.probe('save-editor')
        self.view(host,lambda v:any('changed elsewhere' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('read')['aliasValue'],'draft')
    def test_deleted_item_and_invalid_schema_are_not_overwritten(self):
        host=self.host();self.show(host);self.alias(host)
        config=json.loads(host.config.read_bytes());config['commands']=[c for c in config['commands'] if c['id']!='app.finder'];host.replace(config);before=host.config.read_bytes();host.probe('alias-input',text='mine');host.probe('save-editor')
        self.view(host,lambda v:any('removed' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before)
        host.probe('cancel-editor');self.saved(host);host.replace(DEFAULT)
        self.assertTrue(host.rpc({'op':'reload'})['ok']);self.show(host);self.alias(host)
        config=copy.deepcopy(DEFAULT);config['commands'][0]['unknownField']='PRIVATE-SENTINEL';host.replace(config);before=host.config.read_bytes();host.probe('alias-input',text='mine');host.probe('save-editor')
        self.view(host,lambda v:any('unknown field' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertNotIn('PRIVATE-SENTINEL',json.dumps(host.probe('read')))
    def test_hotkey_sides_hyper_delete_and_repeat(self):
        host=self.host();self.show(host)
        config=json.loads(host.config.read_bytes());config['separateCommandKeys']=True;host.replace(config);host.settled(lambda s:s['separateCommandKeys'])
        self.hotkey(host);before=host.config.read_bytes();self.key(host,43,CMD|SHIFT,',',True);self.assertEqual(host.config.read_bytes(),before)
        self.capture(host,'hotkey-recording');self.key(host,45,CMD|SHIFT|0x10,'different layout');self.saved(host);self.assertEqual(self.command(host)['hotkey'],'shift+right-cmd+n')
        for flags,expected in [(CMD|0x8,'left-cmd+n'),(CMD|CTRL|SHIFT|OPT|0x10,'ctrl+alt+shift+cmd+n')]:
            self.hotkey(host);self.key(host,45,flags,'n');self.saved(host);self.assertEqual(self.command(host)['hotkey'],expected)
        self.hotkey(host);self.key(host,51);self.saved(host);self.assertNotIn('hotkey',self.command(host))
        self.hotkey(host);self.key(host,9,CMD,'v');self.saved(host);self.assertEqual(self.command(host)['hotkey'],'cmd+v')
    def test_hotkey_conflict_and_solo_modifiers(self):
        host=self.host();self.show(host);self.hotkey(host);before=host.config.read_bytes()
        self.key(host,9,CTRL|OPT,'v');self.view(host,lambda v:any('Duplicate hotkey' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertFalse(host.probe('read')['recording']);self.capture(host,'hotkey-conflict')
        for code,flags,expected in [(59,CTRL,'ctrl+left-control'),(58,OPT,'alt+left-option')]:
            if not host.probe('read')['formVisible']:self.hotkey(host)
            else:host.probe('record');self.view(host,lambda v:v['recording'])
            host.probe('flags-event',code=code,flags=flags);host.probe('flags-event',code=code,flags=0);self.saved(host);self.assertEqual(self.command(host)['hotkey'],expected)
    def test_recorder_stops_on_window_app_resign_unfocus_hide_and_cancel(self):
        host=self.host();self.show(host)
        for action in ['resign-window','resign-app','unfocus']:
            self.hotkey(host);before=host.config.read_bytes();host.probe(action);self.assertFalse(host.probe('read')['recording']);host.probe('flags-event',code=59,flags=CTRL);host.probe('flags-event',code=59,flags=0);self.assertEqual(host.config.read_bytes(),before);host.probe('cancel-editor');self.saved(host)
        self.hotkey(host);before=host.config.read_bytes();host.probe('hide-palette');host.settled(lambda s:not s['nativeUI']['visible']);self.assertFalse(host.probe('read')['recording']);host.probe('record');self.assertFalse(host.probe('read')['recording']);self.assertEqual(host.config.read_bytes(),before)
    def test_alias_native_composition_does_not_save_or_cancel(self):
        host=self.host();self.show(host);self.alias(host);before=host.config.read_bytes();token=host.probe('read')['editorToken'];host.probe('alias-ime');view=host.probe('read');self.assertEqual(view['editorToken'],token);self.assertEqual(host.config.read_bytes(),before);host.probe('cancel-editor');self.saved(host)
    def test_remove_confirmation_cancel_then_commit_and_disable(self):
        host=self.host();self.show(host);self.action(host,'Remove');self.view(host,lambda v:v['formVisible']);before=host.config.read_bytes();self.capture(host,'remove-confirmation');host.probe('cancel-editor');self.saved(host);self.assertEqual(host.config.read_bytes(),before)
        self.action(host,'Remove');self.view(host,lambda v:v['formVisible']);host.probe('save-editor');self.saved(host);self.assertFalse(any(c['id']=='app.finder' for c in json.loads(host.config.read_bytes())['commands']))
        self.show(host,'safari');self.action(host,'Disable');self.view(host,lambda v:not v['actionsVisible']);host.settled(lambda s:not s['configurationUpdating']);self.assertFalse(self.command(host,'app.safari')['enabled']);self.assertFalse(any(c['id']=='app.safari' for c in host.cli('search','safari')['result']))
    def test_visible_selection_is_bound_and_unseen_selection_is_refused(self):
        host=self.host();self.show(host,'')
        host.probe('select-and-alias',row=1);self.view(host,lambda v:v['formVisible']);self.assertIn('Safari',host.probe('read')['screenText']);host.probe('cancel-editor');self.saved(host)
        self.show(host,'');host.probe('unseen-selection-and-alias',row=1);self.view(host,lambda v:any('Selection changed' in s for s in v['screenText']));self.assertFalse(host.probe('read')['formVisible'])
    def test_busy_config_lock_keeps_draft_and_retries(self):
        host=self.host();self.show(host);self.alias(host);before=host.config.read_bytes()
        with (host.profile/'config.lock').open('a') as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB);host.probe('alias-input',text='locked-draft');host.probe('save-editor');self.view(host,lambda v:any('lock' in s.lower() or 'busy' in s.lower() for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before)
        host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['aliases'],['locked-draft'])
    def test_queued_save_cancel_and_hide_prevent_dispatch_and_zombie_editor(self):
        host=self.host()
        for mode in ['schedule-save-cancel','schedule-save-hide']:
            self.show(host);self.alias(host);token=host.probe('read')['editorToken'];before=host.config.read_bytes()
            host.probe('alias-input',text='must-not-save');host.probe(mode)
            self.assertTrue(host.rpc({'op':'native.delay'})['ok'])
            view=self.saved(host);self.assertIsNone(view['editorToken']);self.assertEqual(host.config.read_bytes(),before)
            host.settled(lambda s:s['nativeUI']['visible']==(mode=='schedule-save-cancel'))
            self.show(host);self.alias(host);self.assertGreater(host.probe('read')['editorToken'],token);host.probe('cancel-editor');self.saved(host)
    def test_queued_menu_return_cancel_does_not_open_editor(self):
        host=self.host();self.show(host);self.key(host,40,CMD,'k');self.view(host,lambda v:v['actionsVisible'])
        self.key(host,125);self.key(host,125);host.settled()
        host.probe('schedule-menu-cancel');self.assertTrue(host.rpc({'op':'native.delay'})['ok'])
        view=self.view(host,lambda v:not v['actionsVisible']);self.assertFalse(view['formVisible']);self.assertIsNone(view['editorToken'])
        self.alias(host);host.probe('cancel-editor');self.saved(host)
    def test_forms_and_actions_render_with_long_titles_in_light_and_dark(self):
        host=self.host()
        for theme in ['light','dark']:
            config=copy.deepcopy(DEFAULT);config['theme']=theme;config['commands'][0]['title']='Finder — '+('Long Unicode title büro 日本語 · '*7)
            host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled(lambda s:s['theme']==theme)
            self.show(host);self.key(host,40,CMD,'k');self.view(host,lambda v:v['actionsVisible']);self.capture(host,'actions-long-'+theme);self.key(host,53)
            self.alias(host);host.probe('alias-input',text='büro, 文件, myfinder');self.capture(host,'alias-long-'+theme);host.probe('cancel-editor');self.saved(host)
            self.hotkey(host);self.capture(host,'hotkey-long-'+theme);host.probe('cancel-editor');self.saved(host)

if __name__=='__main__':unittest.main(verbosity=2)
