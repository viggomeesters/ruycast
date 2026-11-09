"""Actual native command editor: selected identity, literal fields and persistence."""
import copy
import json
import unittest
import actions_process as helpers
CMD=helpers.CMD
from runtime_process import DEFAULT

class CommandEditor(unittest.TestCase):
    host=helpers.ActionsProcesses.host;capture=helpers.ActionsProcesses.capture;view=helpers.ActionsProcesses.view;show=helpers.ActionsProcesses.show;key=helpers.ActionsProcesses.key;saved=helpers.ActionsProcesses.saved;action=helpers.ActionsProcesses.action;command=helpers.ActionsProcesses.command
    def edit(self,host):
        self.action(host,'Edit');return self.view(host,lambda v:v['formVisible'] and 'Name' in v['screenText'])
    def field(self,host,key,text):host.probe('command-input',key=key,text=text)
    def fixture(self,host,command):
        config=copy.deepcopy(DEFAULT);config['commands'].append(command);host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);self.show(host,command['id'])
    def test_name_alias_and_native_typing_save_and_cancel(self):
        host=self.host();self.show(host);before=host.config.read_bytes();self.edit(host);self.assertNotIn('Enabled',host.probe('read')['screenText']);host.probe('command-type',text='Finder Renamed');self.field(host,'aliases','myfinder, büro');self.capture(host,'edit-app')
        self.key(host,36,CMD);self.saved(host);self.assertEqual(self.command(host)['title'],'Finder Renamed');self.assertEqual(self.command(host)['aliases'],['myfinder','büro'])
        self.edit(host);self.field(host,'title','Must not save');host.probe('cancel-editor');self.saved(host);self.assertEqual(self.command(host)['title'],'Finder Renamed');self.assertNotEqual(host.config.read_bytes(),before)
    def test_validation_preserves_full_draft_and_native_name_focus(self):
        host=self.host();self.show(host);self.edit(host);before=host.config.read_bytes();self.field(host,'aliases','safari');host.probe('save-editor');self.view(host,lambda v:any('duplicate' in t.lower() for t in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.capture(host,'edit-validation')
        host.probe('command-type',text='After rejection');self.assertEqual(host.probe('command-read')['value']['title'],'After rejection');self.field(host,'aliases','repaired');host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['title'],'After rejection')
    def test_external_change_merges_unedited_fields_and_conflict_keeps_draft(self):
        host=self.host();self.show(host);self.edit(host);self.field(host,'title','Local name')
        config=json.loads(host.config.read_bytes());config['commands'][0]['aliases']=['external'];config['clipboard']['limit']=87;host.replace(config);host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['aliases'],['external']);self.assertEqual(json.loads(host.config.read_bytes())['clipboard']['limit'],87)
        self.edit(host);self.field(host,'title','My draft');config=json.loads(host.config.read_bytes());config['commands'][0]['title']='Outside';host.replace(config);before=host.config.read_bytes();host.probe('save-editor');self.view(host,lambda v:any('changed elsewhere' in t for t in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('command-read')['value']['title'],'My draft')
    def test_script_arguments_are_independent_literal_multiline_values(self):
        host=self.host();args=['first\nsecond','$(touch /tmp/never-execute)','büro "quoted"'];self.fixture(host,dict(id='script.fixture',title='Script fixture',kind='script',target='/usr/bin/printf',arguments=args,aliases=[]));self.edit(host)
        host.probe('command-scroll',y=440);self.capture(host,'edit-script-arguments');host.probe('add-argument');host.probe('command-argument',index=3,text='extra\nline');host.probe('remove-argument',index=1)
        self.field(host,'timeout','45.5');host.probe('save-editor');self.saved(host);cmd=self.command(host,'script.fixture');self.assertEqual(cmd['arguments'],[args[0],args[2],'extra\nline']);self.assertEqual(cmd['timeout'],45.5)
    def test_snippet_text_output_keyword_preserved(self):
        host=self.host();self.fixture(host,dict(id='snippet.fixture',title='Snippet fixture',kind='snippet',target='Old text',aliases=[],keyword=';old'));self.edit(host)
        text='Hello [world](https://example.com)\n\nUnicode büro 日本語';self.field(host,'target',text);self.field(host,'snippetOutput','teams');self.field(host,'keyword',';new');self.capture(host,'edit-snippet-top');host.probe('command-scroll',y=200);self.capture(host,'edit-snippet');host.probe('save-editor');self.saved(host);cmd=self.command(host,'snippet.fixture');self.assertEqual(cmd['target'],text);self.assertEqual(cmd['snippetOutput'],'teams');self.assertEqual(cmd['keyword'],';new')
    def test_recording_changes_draft_only_and_escape_stops_without_closing(self):
        host=self.host();self.show(host);self.edit(host);before=host.config.read_bytes();host.probe('record');self.view(host,lambda v:v['recording']);self.key(host,53);self.assertFalse(host.probe('read')['recording']);self.assertTrue(host.probe('read')['formVisible'])
        host.probe('record');self.key(host,45,CMD,'n');self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('command-read')['value']['hotkey'],'cmd+n');host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['hotkey'],'cmd+n')
    def test_quicklink_domain_and_file_url_normalize(self):
        host=self.host();self.fixture(host,dict(id='link.fixture',title='Link fixture',kind='url',target='https://example.com',aliases=[]));self.edit(host);self.field(host,'target','example.org/path');host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host,'link.fixture')['target'],'https://example.org/path')
        self.edit(host);self.field(host,'target','file:///tmp/hello%20world');host.probe('save-editor');self.saved(host);cmd=self.command(host,'link.fixture');self.assertEqual(cmd['kind'],'path');self.assertEqual(cmd['target'],'/tmp/hello world')

    def test_noop_save_preserves_explicit_plain_and_empty_optional_fields(self):
        host=self.host();self.fixture(host,dict(id='snippet.fixture',title='Snippet fixture',kind='snippet',target='Text',snippetOutput='plain',inputPrompt='',aliases=['a,b']));self.edit(host);before=json.loads(host.config.read_bytes());host.probe('save-editor');self.saved(host);self.assertEqual(json.loads(host.config.read_bytes()),before)
    def test_snippet_undo_is_local_to_the_opened_draft(self):
        host=self.host();config=copy.deepcopy(DEFAULT);config['commands'] += [dict(id='snippet.a',title='Snippet A',kind='snippet',target='First',aliases=[]),dict(id='snippet.b',title='Snippet B',kind='snippet',target='Second',aliases=[])];host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
        self.show(host,'snippet.a');self.edit(host);host.probe('snippet-type',text='PRIVATE-DRAFT');self.assertTrue(host.probe('snippet-undo-state')['canUndo']);host.probe('cancel-editor');self.saved(host)
        self.show(host,'snippet.b');self.edit(host);self.assertFalse(host.probe('snippet-undo-state')['canUndo']);self.assertEqual(host.probe('command-read')['value']['target'],'Second')

    def test_native_composition_and_multiline_escape_routes(self):
        host=self.host();self.show(host);self.edit(host);before=host.config.read_bytes();host.probe('command-ime');self.assertTrue(host.probe('read')['formVisible']);self.assertEqual(host.config.read_bytes(),before);host.probe('cancel-editor');self.saved(host)
        for command,argument in [(dict(id='snippet.fixture',title='Snippet',kind='snippet',target='Text',aliases=[]),None),(dict(id='script.fixture',title='Script',kind='script',target='/usr/bin/printf',arguments=['one'],aliases=[]),0)]:
            self.fixture(host,command);self.edit(host);host.probe('editor-text-command',marked=True,argument=argument);self.assertTrue(host.probe('read')['formVisible']);host.probe('editor-text-command',marked=False,argument=argument);self.saved(host)

    def test_alias_form_noop_preserves_literal_comma_alias(self):
        host=self.host();config=copy.deepcopy(DEFAULT);config['commands'][0]['aliases']=['literal,alias'];host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);self.show(host);helpers.ActionsProcesses.alias(self,host);host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['aliases'],['literal,alias'])
    def test_full_editor_queued_save_cancel_does_not_publish(self):
        host=self.host();self.show(host);self.edit(host);before=host.config.read_bytes();self.field(host,'title','Must not publish');host.probe('schedule-save-cancel');self.assertTrue(host.rpc({'op':'native.delay'})['ok']);self.saved(host);self.assertEqual(host.config.read_bytes(),before)

    def test_detached_old_argument_cannot_cancel_a_new_editor(self):
        host=self.host();self.fixture(host,dict(id='script.fixture',title='Script',kind='script',target='/usr/bin/printf',arguments=['one'],aliases=[]));self.edit(host);host.probe('retain-argument');host.probe('cancel-editor');self.saved(host);self.show(host,'finder');self.edit(host);token=host.probe('read')['editorToken'];host.probe('old-text-command');self.assertTrue(host.probe('read')['formVisible']);self.assertEqual(host.probe('read')['editorToken'],token)

    def test_long_editor_heading_and_scrolled_fields_in_light_and_dark(self):
        host=self.host()
        for theme in ['light','dark']:
            config=copy.deepcopy(DEFAULT);config['theme']=theme;config['commands'][0]['title']='Finder — '+('Unicode büro 日本語 '*12);host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled(lambda s:s['theme']==theme);self.show(host);self.edit(host);self.capture(host,'edit-long-'+theme);view=host.probe('command-scroll',y=260);self.assertGreaterEqual(view['commandScrollY'],0);self.assertLessEqual(view['commandScrollY'],view['commandScrollMaxY']);self.capture(host,'edit-bottom-'+theme);host.probe('cancel-editor');self.saved(host)

if __name__=='__main__':unittest.main(verbosity=2)

