"""Own app bundles, real native picker and saved bundle-ID identity."""
import copy
import json
import unittest
import actions_process as actions
import applications_process as apps
import bridge_process as bridge
import runtime_process as rt

class AppPicker(unittest.TestCase):
    view=actions.ActionsProcesses.view
    show=actions.ActionsProcesses.show
    key=actions.ActionsProcesses.key
    action=actions.ActionsProcesses.action
    saved=actions.ActionsProcesses.saved
    capture=actions.ActionsProcesses.capture
    alias=actions.ActionsProcesses.alias
    hotkey=actions.ActionsProcesses.hotkey
    def host(self,application=None,duplicate=False,barrier=False,theme=None,width=760,long=False,empty=False,default_name=False,missing_name=False):
        h=bridge.Bridge(start=False);self.addCleanup(h.close)
        h.env['RUYCAST_NATIVE_TEST_WIDTH']=str(width)
        for identifier,title in ([] if empty else [('dev.fixture.a','Fixture Browser A'),('dev.fixture.b','Fixture Browser B')]):
            apps.Applications.bundle(self,h.applications,identifier,identifier,CFBundleName=('Default app' if default_name and identifier=='dev.fixture.a' else 'dev.fixture.missing' if missing_name and identifier=='dev.fixture.a' else 'Fixture Same' if duplicate else (title+(' long büro 日本語 '*20) if long else title)))
        cfg=copy.deepcopy(rt.DEFAULT);cmd=dict(id='url.fixture',title='Fixture Link',kind='url',target='https://example.invalid/own',aliases=['fixtureurl'])
        if application is not None:cmd['applicationID']=application
        cfg['commands'].append(cmd)
        if theme:cfg['theme']=theme
        h.config.write_text(json.dumps(cfg))
        if barrier:h.env['RUYCAST_NATIVE_TEST_APP_BARRIER']='1'
        h.start()
        if not barrier:h.wait(lambda s:not s['appDiscoveryPending'])
        h.settled();return h
    def edit(self,h):
        self.show(h,'fixtureurl');self.action(h,'Edit');self.view(h,lambda v:v['formVisible'] and 'Name' in v['screenText']);h.settled()
        return h.probe('app-picker',command='read')
    def read(self,h):h.settled();return h.probe('app-picker',command='read')
    def choose(self,h,index):
        h.probe('app-picker',command='open');self.assertTrue(self.read(h)['list']['open'])
        h.probe('app-picker',command='select',index=index);self.key(h,36,0,'\r');h.settled()
    def command(self,h):return next(c for c in json.loads(h.config.read_text())['commands'] if c['id']=='url.fixture')
    def test_long_app_titles_native_accessibility_and_light_dark_widths(self):
        for theme in ['light','dark']:
            for width in [600,760]:
                h=self.host(theme=theme,width=width,long=True);self.edit(h);self.choose(h,2);v=self.read(h)
                self.assertEqual(v['axValue'],'dev.fixture.b');self.assertGreater(len(v['title']),200)
                self.capture(h,'app-long-'+theme+'-'+str(width));h.probe('app-picker',command='open');view=self.capture(h,'app-long-open-'+theme+'-'+str(width))
                self.assertIn(v['title'],view['accessibilityText']);self.key(h,53);h.probe('cancel-editor');self.saved(h)
    def test_empty_discovery_keeps_default_and_tab_closes_tentative_choice(self):
        h=self.host(empty=True);self.edit(h);h.probe('app-picker',command='open');self.assertEqual(self.read(h)['list']['rows'],1)
        self.capture(h,'app-empty');self.key(h,48,0,'\t');self.assertFalse(self.read(h)['list']['open']);self.assertEqual(self.read(h)['value'],'');self.assertFalse(self.read(h)['fieldAXHidden'])

    def test_explicit_empty_id_is_preserved_until_default_is_explicitly_chosen(self):
        h=self.host('');self.edit(h);h.probe('command-input',key='title',text='Only title changed');h.probe('save-editor');self.saved(h)
        self.assertIn('applicationID',self.command(h));self.assertEqual(self.command(h)['applicationID'],'')
        self.edit(h);self.choose(h,0);h.probe('save-editor');self.saved(h);self.assertNotIn('applicationID',self.command(h))
    def test_reserved_default_name_and_saved_id_are_visibly_distinct(self):
        h=self.host(default_name=True);self.edit(h);h.probe('app-picker',command='open');view=self.capture(h,'app-reserved-default')
        self.assertIn('Default app',view['accessibilityText']);self.assertIn('Default app — dev.fixture.a',view['accessibilityText'])
        h.probe('app-picker',command='click',index=1);self.assertEqual(self.read(h)['value'],'dev.fixture.a')
        h=self.host('dev.fixture.missing',missing_name=True);self.edit(h);h.probe('app-picker',command='open');view=self.capture(h,'app-saved-id')
        self.assertIn('Saved app — dev.fixture.missing',view['accessibilityText']);self.assertIn('dev.fixture.missing',view['accessibilityText'])
        h.probe('app-picker',command='click',index=1);self.assertEqual(self.read(h)['value'],'dev.fixture.missing')
    def test_modal_keys_cancel_and_stale_callbacks_do_not_change_draft(self):
        h=self.host();self.edit(h);before=h.config.read_bytes()
        h.probe('app-picker',command='open');h.probe('app-picker',command='retain');old=self.read(h)
        self.assertTrue(old['expanded']);self.assertTrue(old['fieldAXHidden']);self.assertFalse(h.probe('read')['saveEnabled'])
        h.probe('app-picker',command='select',index=2);self.key(h,36,actions.CMD,'\r');h.probe('save-editor')
        self.assertTrue(self.read(h)['list']['open']);self.assertEqual(self.read(h)['value'],'')
        self.key(h,53);self.assertFalse(self.read(h)['fieldAXHidden']);self.assertEqual(self.read(h)['value'],'');self.assertTrue(h.probe('read')['saveEnabled'])
        h.probe('app-picker',command='open');current=self.read(h);self.assertGreater(current['list']['generation'],old['list']['generation'])
        h.probe('app-picker',command='stale',index=2);self.assertEqual(self.read(h)['value'],'');self.assertTrue(self.read(h)['list']['open'])
        self.key(h,53);h.probe('cancel-editor');self.saved(h);self.edit(h)
        token=self.read(h)['token'];h.probe('app-picker',command='stale-button');self.assertEqual(self.read(h)['token'],token);self.assertFalse(self.read(h)['list']['open']);self.assertEqual(h.config.read_bytes(),before)
    def test_late_discovery_and_unrelated_reload_preserve_editor_draft(self):
        h=self.host(barrier=True);v=self.edit(h);self.assertTrue(v['pending']);self.assertFalse(v['enabled'])
        h.probe('command-input',key='title',text='Kept draft')
        (h.base/'app-discovery-release').write_text('go');h.wait(lambda s:not s['appDiscoveryPending']);h.settled()
        self.assertEqual(h.probe('command-read')['value']['title'],'Kept draft');self.assertTrue(self.read(h)['enabled'])
        h.probe('app-picker',command='open');old=self.read(h);cfg=json.loads(h.config.read_text());cfg['theme']='light';h.replace(cfg);h.cli('reload');h.settled()
        self.assertEqual(self.read(h)['list']['generation'],old['list']['generation']);self.assertTrue(self.read(h)['fieldAXHidden'])
        self.key(h,53);self.assertEqual(h.probe('command-read')['value']['title'],'Kept draft')
    def test_external_application_id_conflict_keeps_draft_and_file(self):
        h=self.host();self.edit(h);self.choose(h,2)
        cfg=json.loads(h.config.read_text());next(c for c in cfg['commands'] if c['id']=='url.fixture')['applicationID']='dev.fixture.a';h.replace(cfg);before=h.config.read_bytes()
        h.probe('save-editor');self.view(h,lambda v:any('changed elsewhere' in t for t in v['screenText']))
        self.assertEqual(self.read(h)['value'],'dev.fixture.b');self.assertEqual(h.config.read_bytes(),before)
    def test_discovered_app_alias_hotkey_edit_and_disable_persist(self):
        h=self.host();self.show(h,'Fixture Browser A');self.alias(h);h.probe('alias-input',text='ownbrowser');h.probe('save-editor');self.saved(h)
        cfg=json.loads(h.config.read_text());cmd=next(c for c in cfg['commands'] if c['target']=='dev.fixture.a');self.assertEqual(cmd['aliases'],['ownbrowser'])
        self.show(h,'ownbrowser');self.hotkey(h);self.key(h,18,actions.CMD|actions.SHIFT,'1');self.saved(h)
        cmd=next(c for c in json.loads(h.config.read_text())['commands'] if c['target']=='dev.fixture.a');self.assertEqual(cmd['hotkey'],'shift+cmd+1')
        self.show(h,'Fixture Browser B');self.action(h,'Edit');self.view(h,lambda v:v['formVisible']);h.probe('command-input',key='title',text='Own browser B');h.probe('save-editor');self.saved(h)
        self.show(h,'Own browser B');self.action(h,'Disable');h.settled();cmd=next(c for c in json.loads(h.config.read_text())['commands'] if c['target']=='dev.fixture.b');self.assertFalse(cmd['enabled'])
    def test_discovered_app_disable_creates_suppression_without_alias(self):
        h=self.host();self.show(h,'Fixture Browser B');self.action(h,'Disable');h.settled();cmd=next(c for c in json.loads(h.config.read_text())['commands'] if c['target']=='dev.fixture.b');self.assertFalse(cmd['enabled'])
        self.assertFalse(any(c['target']=='dev.fixture.b' for c in h.rpc({'op':'search','query':'Fixture Browser B'})['result']))

if __name__=='__main__':unittest.main(verbosity=2)
