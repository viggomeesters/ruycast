"""Create commands through actual hidden native Actions and validated drafts."""
import copy
import json
import unittest
import actions_process as helpers
from runtime_process import DEFAULT

class Creation(unittest.TestCase):
    host=helpers.ActionsProcesses.host;capture=helpers.ActionsProcesses.capture;view=helpers.ActionsProcesses.view;show=helpers.ActionsProcesses.show;key=helpers.ActionsProcesses.key;saved=helpers.ActionsProcesses.saved;action=helpers.ActionsProcesses.action
    def field(self,host,key,text):host.probe('command-input',key=key,text=text)
    def rows(self,host):return json.loads(host.config.read_bytes())['commands']
    def new(self,host,title):
        self.action(host,title);self.view(host,lambda v:v['formVisible'] and 'Name' in v['screenText']);return host.probe('command-read')['value']
    def test_empty_results_keep_all_three_creation_actions_available(self):
        host=self.host();self.show(host,'not-found-Ω');before=host.config.read_bytes();self.key(host,40,helpers.CMD,'k');v=self.view(host,lambda v:v['actionsVisible']);self.assertEqual(v['selectedRow'],-1)
        for name in ['New Snippet…','New Quicklink…','New Shell Command…']:self.assertIn(name,v['actionLabels'])
        self.capture(host,'create-empty-menu');self.new(host,'New Snippet');host.probe('cancel-editor');self.saved(host);self.assertEqual(host.config.read_bytes(),before)
    def test_snippet_is_created_only_after_save_and_resolves_by_alias(self):
        host=self.host();self.show(host);before=host.config.read_bytes();count=len(self.rows(host));draft=self.new(host,'New Snippet');self.assertTrue(draft['id'].startswith('snippet.'));self.assertEqual(host.config.read_bytes(),before)
        self.field(host,'title','My Snippet');self.field(host,'target','Hello\nUnicode büro');self.field(host,'aliases','mynew');self.field(host,'keyword',';new');self.field(host,'snippetOutput','teams');self.capture(host,'create-snippet')
        host.probe('save-editor');self.saved(host);rows=self.rows(host);self.assertEqual(len(rows),count+1);created=next(c for c in rows if c['id']==draft['id']);self.assertEqual(created['target'],'Hello\nUnicode büro');self.assertEqual(created['snippetOutput'],'teams');self.assertEqual(host.cli('search','mynew')['result'][0]['id'],draft['id']);host.probe('save-editor');self.assertEqual(len(self.rows(host)),count+1)
    def test_validation_retains_new_identity_then_repairs(self):
        host=self.host();self.show(host);before=host.config.read_bytes();draft=self.new(host,'New Snippet');host.probe('save-editor');self.view(host,lambda v:any('need id, title and target' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('command-read')['value']['id'],draft['id']);self.field(host,'target','Valid');host.probe('save-editor');self.saved(host);self.assertEqual(self.rows(host)[-1]['id'],draft['id'])
    def test_quicklinks_normalize_domain_and_file_path_without_hidden_general_clipboard(self):
        host=self.host();self.show(host);draft=self.new(host,'New Quicklink');self.assertEqual(draft['target'],'https://');self.field(host,'target','example.org/path');host.probe('save-editor');self.saved(host);self.assertEqual(self.rows(host)[-1]['target'],'https://example.org/path')
        draft=self.new(host,'New Quicklink');self.field(host,'target','file:///tmp/hello%20world');host.probe('save-editor');self.saved(host);created=next(c for c in self.rows(host) if c['id']==draft['id']);self.assertEqual(created['kind'],'path');self.assertEqual(created['target'],'/tmp/hello world')
    def test_shell_command_arguments_and_draft_hotkey_are_literal(self):
        host=self.host();self.show(host);before=host.config.read_bytes();draft=self.new(host,'New Shell Command');host.probe('add-argument');host.probe('command-argument',index=0,text='one\ntwo $(no expansion)');host.probe('record');self.view(host,lambda v:v['recording']);self.key(host,45,helpers.CMD,'n');self.assertEqual(host.config.read_bytes(),before);host.probe('save-editor');self.saved(host);created=next(c for c in self.rows(host) if c['id']==draft['id']);self.assertEqual(created['arguments'],['one\ntwo $(no expansion)']);self.assertEqual(created['hotkey'],'cmd+n')
    def test_external_changes_survive_and_identity_collision_never_overwrites(self):
        host=self.host();self.show(host);draft=self.new(host,'New Shell Command');config=json.loads(host.config.read_bytes());config['clipboard']['limit']=92;host.replace(config);host.probe('save-editor');self.saved(host);self.assertEqual(json.loads(host.config.read_bytes())['clipboard']['limit'],92)
        draft=self.new(host,'New Shell Command');config=json.loads(host.config.read_bytes());external=copy.deepcopy(draft);external['title']='External owner';config['commands'].append(external);host.replace(config);before=host.config.read_bytes();host.probe('save-editor');self.view(host,lambda v:any('identity already exists' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before)
    def test_full_creation_menu_light_dark_and_maximum_actions_fit(self):
        host=self.host();config=copy.deepcopy(DEFAULT);config['commands'].append(dict(id='snippet.fixture',title='Snippet fixture',kind='snippet',target='Text',aliases=[]))
        for theme in ['light','dark']:
            config['theme']=theme;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled(lambda s:s['theme']==theme);self.show(host,'snippet.fixture');self.key(host,40,helpers.CMD,'k');v=self.view(host,lambda v:v['actionsVisible']);self.assertEqual(len(v['actionLabels']),11);self.capture(host,'create-full-menu-'+theme);self.key(host,53)
    def test_queued_creation_menu_cancel_during_busy_owner_stays_cancelled(self):
        host=self.host();self.show(host);before=host.config.read_bytes();self.key(host,40,helpers.CMD,'k');v=self.view(host,lambda v:v['actionsVisible']);index=v['actionLabels'].index('New Snippet…')
        for _ in range(index):self.key(host,125)
        host.settled();host.probe('schedule-menu-cancel');self.assertTrue(host.rpc({'op':'native.delay'})['ok']);v=self.view(host,lambda v:not v['actionsVisible']);self.assertFalse(v['formVisible']);self.assertEqual(host.config.read_bytes(),before)

    def test_menu_escape_during_in_progress_prefill_does_not_open_a_zombie_editor(self):
        host=self.host();self.show(host);before=host.config.read_bytes();self.key(host,40,helpers.CMD,'k');v=self.view(host,lambda v:v['actionsVisible']);index=v['actionLabels'].index('New Quicklink…');host.probe('arm-slow-prefill');host.probe('schedule-menu-escape');host.probe('action-button',index=index)
        host.settled();v=self.view(host,lambda v:not v['actionsVisible']);self.assertEqual(v['prefillTestRuns'],1);self.assertFalse(v['formVisible']);self.assertEqual(host.config.read_bytes(),before);self.new(host,'New Snippet');host.probe('cancel-editor');self.saved(host)
    def start_slow(self,host):
        self.show(host);self.key(host,40,helpers.CMD,'k');v=self.view(host,lambda v:v['actionsVisible']);host.probe('arm-slow-prefill');host.probe('action-button',index=v['actionLabels'].index('New Quicklink…'))
        host.wait(lambda s:s['nativeUI']['creationPreparing']);self.assertFalse(host.probe('read')['formVisible'])
    def test_pending_prefill_keeps_owner_responsive_and_new_query_retires_draft(self):
        host=self.host();before=host.config.read_bytes();self.start_slow(host)
        self.assertTrue(host.rpc({'op':'commands'})['ok']);self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['creationPreparing'])
        host.probe('query',query='safari');host.wait(lambda s:not s['nativeUI']['creationPreparing']);host.settled()
        self.assertFalse(host.probe('read')['formVisible']);self.assertEqual(host.config.read_bytes(),before)
        self.show(host);self.new(host,'New Snippet');host.probe('cancel-editor');self.saved(host)
    def test_pending_prefill_config_change_cancels_without_a_late_form(self):
        host=self.host();self.start_slow(host);cfg=json.loads(host.config.read_text());cfg['theme']='light';host.replace(cfg);host.cli('reload')
        host.wait(lambda s:not s['nativeUI']['creationPreparing']);host.settled();self.assertFalse(host.probe('read')['formVisible']);self.assertEqual(json.loads(host.config.read_text()),cfg)
    def test_pending_prefill_does_not_hold_shutdown_open(self):
        host=self.host();self.start_slow(host);self.assertTrue(host.rpc({'op':'quit'})['ok']);out,err=host.process.communicate(timeout=3);self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
    def test_cancelled_origin_filters_an_already_published_creation_view(self):
        host=self.host();self.show(host);before=host.config.read_bytes();self.new(host,'New Snippet');host.probe('cancel-creation-origin');self.saved(host);self.assertEqual(host.config.read_bytes(),before)

if __name__=='__main__':unittest.main(verbosity=2)
