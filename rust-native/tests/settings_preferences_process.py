"""Actual hidden management controls and short owned profiles. No global input."""
import copy,json,os,shutil,time,unittest
from pathlib import Path
from library_process import SettingsHost,Palette
from runtime_process import DEFAULT
class Preferences(unittest.TestCase):
    def host(self):
        h=SettingsHost(start=False);self.addCleanup(h.close);c=copy.deepcopy(DEFAULT);c['snippetExpansionEnabled']=False;h.replace(c);h.start();h.settled();h.rpc(dict(op='show',mode='settings'));h.settled();return h
    def nav(self,h,tab):h.probe('library-tab',tab=tab);h.settled();return h.probe('settings-read')
    def wait(self,h,predicate):
        end=time.monotonic()+5
        while time.monotonic()<end:
            h.settled();v=h.probe('settings-read')
            if predicate(v):return v
            time.sleep(.02)
        self.fail(v)
    def capture(self,h,name):
        v=h.probe('settings-screenshot')
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            folder=Path(folder);folder.mkdir(parents=True,exist_ok=True);shutil.copy2(h.profile/'native-settings-preferences-probe.png',folder/(name+'.png'));(folder/(name+'.json')).write_text(json.dumps(v,indent=2,ensure_ascii=False))
        return v
    def test_general_actual_controls_write_config_preserve_unknown_fields_and_themes(self):
        h=self.host();v=self.nav(h,'general');self.assertTrue(v['visible']);self.assertIn('Open Launcher',v['text']);before=json.loads(h.config.read_text())
        self.assertTrue(all(b['role'] in ['AXCheckBox','AXButton'] for b in v['buttons']))
        for index,key,default in [(0,'hyperKeyEnabled',False),(1,'separateCommandKeys',True),(2,'menuBarVisible',True)]:
            h.probe('settings-control',index=index);self.wait(h,lambda v:v['surface']['configuration'].get(key,default)!=before.get(key,default))
        for index,theme in [(1,'light'),(2,'dark'),(3,'graphite'),(0,'system')]:
            h.probe('settings-theme',index=index);self.wait(h,lambda v:v['surface']['configuration']['theme']==theme)
            for width,height in [(680,480),(980,680)]:h.probe('dashboard-size',width=width,height=height);self.capture(h,f'general-{theme}-{width}-{height}')
        after=json.loads(h.config.read_text());self.assertEqual(after['commands'],before['commands']);self.assertEqual(after['snippetExpansionEnabled'],False);self.assertFalse(h.probe('read')['windowShown'])
    def test_record_both_shortcuts_local_events_cancel_and_navigation_release(self):
        h=self.host();self.nav(h,'general')
        for index,key,code,chars in [(0,'launcherHotkey',0,'a'),(1,'clipboardHotkey',11,'b')]:
            h.probe('settings-record',index=index);v=self.wait(h,lambda v:v['recording'][index]);self.assertFalse(any(v['preparing']))
            h.probe('key-event',code=code,flags=(1<<20)|(1<<19),chars=chars)
            self.wait(h,lambda v:v['surface']['configuration'][key]==f'alt+cmd+{chars}')
        h.probe('settings-record',index=0);self.wait(h,lambda v:v['recording'][0]);before=h.config.read_bytes();h.probe('key-event',code=53,flags=0,chars='');self.wait(h,lambda v:not any(v['recording']));self.assertEqual(h.config.read_bytes(),before)
        h.probe('settings-record',index=1);self.wait(h,lambda v:v['recording'][1]);self.nav(h,'commands');self.assertFalse(any(h.probe('settings-read')['recording']))
        self.nav(h,'general');h.probe('settings-retain',index=0);before=h.config.read_bytes();h.probe('window-close');h.settled(lambda s:not s['managementUI']['visible']);h.rpc(dict(op='show',mode='settings'));h.settled();self.nav(h,'general');h.probe('settings-stale');h.settled();self.assertEqual(h.config.read_bytes(),before)
    def test_advanced_reload_open_permissions_and_fake_folder_cancel_stale_guard(self):
        h=self.host();v=self.nav(h,'advanced');self.assertIn('Ruycast '+h.rpc(dict(op='status'))['result']['version'],v['text']);h.probe('dashboard-size',width=680,height=480);self.capture(h,'advanced-680-480')
        h.probe('settings-control',index=0);v=self.wait(h,lambda v:v['opened']);self.assertEqual(v['opened'][-1],str(h.config))
        h.probe('settings-control',index=3);v=self.wait(h,lambda v:len(v['opened'])==2);self.assertIn('Privacy_Accessibility',v['opened'][-1])
        h.probe('settings-control',index=2);h.settled();self.assertFalse(h.probe('settings-read')['surface']['busy'])
        h.probe('settings-control',index=1);v=h.probe('settings-read');self.assertTrue(v['fixtureDialog']);self.assertTrue(h.probe('settings-folder-complete',serial=v['dialogSerial'],accepted=False)['accepted'])
        h.probe('settings-control',index=1);v=h.probe('settings-read');self.nav(h,'commands');self.nav(h,'advanced');target=h.base/'moved';target.mkdir();self.assertFalse(h.probe('settings-folder-complete',serial=v['dialogSerial'],accepted=True,path=str(target))['accepted']);self.assertFalse((target/'config.json').exists())
        h.probe('settings-control',index=1);v=h.probe('settings-read');self.assertTrue(h.probe('settings-folder-complete',serial=v['dialogSerial'],accepted=True,path=str(target))['accepted']);self.wait(h,lambda v:v['surface']['directory']==str(target));self.assertTrue((target/'config.json').exists());self.assertTrue(h.config.exists())
    def test_menu_bar_hidden_substitution_live_visibility_and_retired_callbacks(self):
        h=self.host();p=Palette(h);m=p.probe('menu-bar-read');self.assertTrue(m['hiddenSubstitution']);self.assertEqual(m['nativeRegistrations'],0);self.assertEqual(m['items'],['Open Launcher','Settings…','Quit']);p.probe('menu-bar-retain')
        self.nav(h,'general');h.probe('settings-control',index=2);self.wait(h,lambda v:not v['surface']['configuration']['menuBarVisible']);h.probe('window-close');h.settled();p.probe('menu-bar-stale');h.settled();self.assertFalse(h.rpc(dict(op='status'))['result']['nativeUI']['visible'])
        h.rpc(dict(op='show',mode='settings'));h.settled();self.nav(h,'general');h.probe('settings-control',index=2);self.wait(h,lambda v:v['surface']['configuration']['menuBarVisible']);p.probe('menu-bar-click',index=0);h.wait(lambda s:s['nativeUI']['visible']);self.assertFalse(p.probe('read')['windowShown']);p.probe('menu-bar-click',index=1);h.settled();self.assertTrue(h.rpc(dict(op='status'))['result']['managementUI']['visible'])
if __name__=='__main__':unittest.main(verbosity=2)
