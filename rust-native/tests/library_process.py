"""Separate native Settings and palette, one private worker-owned configuration."""
import copy
import json
import os
from pathlib import Path
import shutil
import unittest
import bridge_process as bridge
import actions_process as helpers
import editor_process as editor
from runtime_process import DEFAULT

class SettingsHost(bridge.Bridge):
    def probe(self, action, **fields):
        value=self.rpc(dict(op='native.test',surface='settings',action=action,**fields))
        if not value.get('ok'):raise AssertionError(value)
        return value['result']
    def settled(self, predicate=lambda value:True):
        return self.wait(lambda s:not s.get('configurationUpdating',False) and s.get('managementUI',{}).get('revision',0)>0 and s['managementUI']['revision']==s['managementUI']['appliedRevision'] and predicate(s))

class Palette:
    def __init__(self, host):self.host=host
    def __getattr__(self, key):return getattr(self.host,key)
    def probe(self, action, **fields):return bridge.Bridge.probe(self.host,action,**fields)
    def settled(self,predicate=lambda value:True):return bridge.Bridge.settled(self.host,predicate)

class Library(unittest.TestCase):
    view=helpers.ActionsProcesses.view
    key=helpers.ActionsProcesses.key
    saved=helpers.ActionsProcesses.saved
    action=helpers.ActionsProcesses.action
    alias=helpers.ActionsProcesses.alias
    edit=editor.CommandEditor.edit
    field=editor.CommandEditor.field
    command=helpers.ActionsProcesses.command
    def host(self):
        host=SettingsHost();self.addCleanup(host.close);host.settled();return host
    def show(self,host,query='finder'):
        self.assertTrue(host.rpc(dict(op='show',mode='settings',query=query))['ok']);host.settled(lambda s:s['managementUI']['visible']);self.view(host,lambda v:v['input']==query)
    def fixture(self,host,commands):
        config=copy.deepcopy(DEFAULT);config['commands']=commands;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled()
    def capture(self,host,name):
        value=host.probe('screenshot');image=host.profile/'native-settings-probe.png';self.assertTrue(image.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            output=Path(evidence).parent/'library';output.mkdir(parents=True,exist_ok=True);shutil.copy2(image,output/(name+'.png'));(output/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value
    def rows(self,host):return host.probe('library-read')['rows']
    def nav(self,host,action,**fields):host.probe(action,**fields);host.settled();return host.probe('library-read')
    def test_cli_settings_is_separate_and_reusable_without_showing_a_window(self):
        host=self.host();self.assertTrue(host.cli('show','settings')['ok']);host.settled(lambda s:s['managementUI']['visible']);self.assertFalse(host.rpc({'op':'status'})['result']['nativeUI']['visible']);self.assertFalse(host.probe('read')['windowShown']);self.assertIn('Commands',host.probe('read')['screenText']);self.capture(host,'settings-library')
        host.probe('window-close');host.settled(lambda s:not s['managementUI']['visible']);self.show(host);self.assertFalse(host.probe('read')['windowShown'])
    def test_disabled_items_can_be_found_and_enabled_without_execution(self):
        host=self.host();commands=copy.deepcopy(DEFAULT['commands']);commands[0]['enabled']=False;self.fixture(host,commands);self.show(host,'finder');self.assertFalse(any(c['id']=='app.finder' for c in host.cli('search','finder')['result']));self.assertEqual([r['id'] for r in self.rows(host) if not r['group']],['app.finder'])
        self.key(host,40,helpers.CMD,'k');v=self.view(host,lambda v:v['actionsVisible']);self.assertIn('Enable',v['actionLabels']);self.assertFalse(any(s.startswith('Open') for s in v['actionLabels']));self.key(host,53);self.nav(host,'library-filter',filter='disabled');self.capture(host,'disabled');self.action(host,'Enable');self.view(host,lambda v:not v['actionsVisible']);host.settled();self.assertTrue(self.command(host)['enabled']);self.assertTrue(any(c['id']=='app.finder' for c in host.cli('search','finder')['result']));self.assertEqual([r for r in self.rows(host) if not r['group']],[])
    def test_tabs_groups_filters_and_query_reset_keep_all_saved_items(self):
        host=self.host();commands=copy.deepcopy(DEFAULT['commands'])+[dict(id='snippet.test',title='Test snippet',kind='snippet',target='Text',aliases=['snippetfind']),dict(id='link.test',title='Test link',kind='url',target='https://example.org',aliases=['linkfind'])];self.fixture(host,commands);self.show(host,'')
        row=next(i for i,r in enumerate(self.rows(host)) if r['group'] and 'Applications' in r['title']);host.probe('click',row=row);host.settled();self.assertFalse(any(r['id']=='app.finder' for r in self.rows(host)));host.probe('query',query='finder');host.settled();self.assertTrue(any(r['id']=='app.finder' for r in self.rows(host)));host.probe('query',query='');host.settled();self.assertFalse(any(r['id']=='app.finder' for r in self.rows(host)))
        self.assertEqual(self.nav(host,'library-tab',tab='snippets')['query'],'');self.assertEqual([r['id'] for r in self.rows(host) if not r['group']],['snippet.test']);host.probe('query',query='snippetfind');host.settled();self.nav(host,'library-tab',tab='quicklinks');self.assertEqual([r['id'] for r in self.rows(host) if not r['group']],['path.documents','link.test']);self.nav(host,'library-tab',tab='snippets');host.probe('query',query='snippetfind');host.settled();self.assertEqual(len([r for r in self.rows(host) if not r['group']]),1)
    def test_library_has_no_launcher_hundred_item_cap(self):
        host=self.host();commands=[dict(id=f'url.{i:04}',title=f'Item {i:04}',kind='url',target=f'https://example.org/{i}',aliases=[f'alias{i}']) for i in range(5000)];self.fixture(host,commands);self.show(host,'');self.assertEqual(len([r for r in self.rows(host) if not r['group'] and not r['id'].startswith('builtin.')]),5000);host.probe('query',query='alias4999');host.settled();self.assertEqual([r['id'] for r in self.rows(host) if not r['group']],['url.4999']);self.capture(host,'last-of-5000')
    def test_different_window_drafts_merge_and_keep_each_others_focus(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.alias(palette);palette.probe('alias-input',text='myfinder');token=palette.probe('read')['editorToken'];self.show(host,'safari');self.edit(host);self.field(host,'title','Safari renamed');host.probe('save-editor');self.saved(host);palette.settled();self.assertEqual(palette.probe('read')['editorToken'],token);self.assertEqual(palette.probe('read')['aliasValue'],'myfinder');palette.probe('save-editor');self.saved(palette);self.assertEqual(self.command(host)['aliases'],['myfinder']);self.assertEqual(self.command(host,'app.safari')['title'],'Safari renamed')
    def test_same_field_conflict_retains_second_draft(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.edit(palette);self.field(palette,'title','First window');self.show(host);self.edit(host);self.field(host,'title','Second window');palette.probe('save-editor');self.saved(palette);host.settled();before=host.config.read_bytes();host.probe('save-editor');self.view(host,lambda v:any('changed elsewhere' in s for s in v['screenText']));self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('command-read')['value']['title'],'Second window')
    def test_cmd_w_closes_only_settings_and_discards_only_its_draft(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.alias(palette);palette.probe('alias-input',text='keepmine');token=palette.probe('read')['editorToken'];self.show(host,'safari');self.edit(host);self.field(host,'title','Discard');before=host.config.read_bytes();self.key(host,13,helpers.CMD,'w');host.settled(lambda s:not s['managementUI']['visible'] and s['nativeUI']['visible']);self.assertEqual(host.config.read_bytes(),before);self.assertEqual(palette.probe('read')['editorToken'],token);self.assertEqual(palette.probe('read')['aliasValue'],'keepmine');self.show(host,'safari');self.assertFalse(host.probe('read')['formVisible'])
    def test_repeated_show_preserves_actual_alias_and_name_editor_focus(self):
        host=self.host();self.show(host);self.alias(host);host.probe('alias-input',text='my');token=host.probe('read')['editorToken'];self.assertTrue(host.rpc(dict(op='show',mode='settings'))['ok']);host.settled();self.assertEqual(host.probe('read')['editorToken'],token);host.probe('alias-type',text='x');self.assertIn('x',host.probe('read')['aliasValue']);host.probe('cancel-editor');self.saved(host);self.edit(host);self.assertTrue(host.rpc(dict(op='show',mode='settings'))['ok']);host.settled();host.probe('command-type',text='Renamed');self.assertIn('Renamed',host.probe('command-read')['value']['title'])
    def test_cmd_w_is_recorded_before_window_close_and_repeated_show_keeps_recording(self):
        host=self.host();self.show(host);self.edit(host);before=host.config.read_bytes();host.probe('record');self.view(host,lambda v:v['recording']);self.assertTrue(host.rpc(dict(op='show',mode='settings'))['ok']);host.settled();self.assertTrue(host.probe('read')['recording']);self.key(host,13,helpers.CMD,'w');self.assertTrue(host.rpc({'op':'status'})['result']['managementUI']['visible']);self.assertFalse(host.probe('read')['recording']);self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('command-read')['value']['hotkey'],'cmd+w');host.probe('save-editor');self.saved(host);self.assertEqual(self.command(host)['hotkey'],'cmd+w')
    def test_toolbar_add_is_private_until_save_then_shared_with_palette(self):
        host=self.host();self.show(host,'');before=host.config.read_bytes();host.probe('library-add',kind='snippet');self.view(host,lambda v:v['formVisible']);self.assertEqual(host.config.read_bytes(),before);self.field(host,'title','Created in Settings');self.field(host,'target','Hello');self.field(host,'aliases','newsettings');id=host.probe('command-read')['value']['id'];self.capture(host,'new-snippet');host.probe('save-editor');self.saved(host);self.assertEqual(host.cli('search','newsettings')['result'][0]['id'],id)
    def test_invalid_config_keeps_both_last_good_views_and_refuses_draft_save(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.show(host);self.edit(host);self.field(host,'title','Keep draft');host.config.write_text('broken PRIVATE-SENTINEL');host.settled(lambda s:s['configError'] is not None);before=host.config.read_bytes();host.probe('save-editor');self.view(host,lambda v:v['formVisible'] and len(v['screenText'])>4);self.assertEqual(host.config.read_bytes(),before);self.assertEqual(host.probe('command-read')['value']['title'],'Keep draft');self.assertNotIn('PRIVATE-SENTINEL',json.dumps(host.probe('read')));self.assertTrue(palette.probe('read')['rows']>0)
    def test_theme_and_application_resign_update_both_windows(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.show(host)
        for theme in ['light','dark']:
            config=json.loads(host.config.read_bytes());config['theme']=theme;host.replace(config);host.settled(lambda s:s['theme']==theme);palette.settled();self.assertEqual('DarkAqua' in host.probe('read')['appearance'],theme=='dark');self.assertEqual('DarkAqua' in palette.probe('read')['appearance'],theme=='dark');self.capture(host,'library-'+theme)
        helpers.ActionsProcesses.hotkey(self,palette);helpers.ActionsProcesses.hotkey(self,host);host.probe('resign-app');self.assertFalse(host.probe('read')['recording']);self.assertFalse(palette.probe('read')['recording']);self.assertTrue(palette.probe('read')['formVisible']);self.assertTrue(host.probe('read')['formVisible'])
    def test_hidden_settings_callbacks_do_not_change_navigation_or_cancel_palette(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.alias(palette);token=palette.probe('read')['editorToken'];self.show(host,'safari');host.probe('window-close');host.settled(lambda s:not s['managementUI']['visible']);before=host.probe('library-read');host.probe('library-tab',tab='snippets');host.probe('library-filter',filter='disabled');host.settled();after=host.probe('library-read');self.assertEqual(after['tab'],before['tab']);self.assertEqual(after['filter'],before['filter']);self.assertFalse(after['visible']);self.assertEqual(palette.probe('read')['editorToken'],token)
    def test_normal_quit_with_both_windows_and_drafts_releases_socket(self):
        host=self.host();palette=Palette(host);helpers.ActionsProcesses.show(self,palette,'finder');self.alias(palette);self.show(host,'safari');self.edit(host);before=host.config.read_bytes();self.assertTrue(host.rpc({'op':'quit'})['ok']);out,err=host.process.communicate(timeout=4);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());self.assertFalse((host.profile/'control.sock').exists());self.assertEqual(host.config.read_bytes(),before)

if __name__=='__main__':unittest.main(verbosity=2)
