"""Actual hidden management controller and owned filesystem; no OS save panels."""
import copy,json,os,shutil,time,unittest
from pathlib import Path
from library_process import SettingsHost
from runtime_process import DEFAULT

class Dashboard(unittest.TestCase):
    def host(self,initial=None,theme=None,counts=None):
        h=SettingsHost(start=False);self.addCleanup(h.close)
        if initial is not None:(h.profile/'analytics.json').write_text(json.dumps(initial))
        if counts is not None:(h.profile/'usage.json').write_text(json.dumps(counts))
        c=copy.deepcopy(DEFAULT)
        c['commands'].append(dict(id='analytics.fixture',title='Own analytics fixture',kind='script',target='/usr/bin/true'))
        c['commands'].append(dict(id='analytics.failure',title='Own failure fixture',kind='script',target='/usr/bin/false'))
        if theme:c['theme']=theme
        h.replace(c)
        h.start();h.settled();return h
    def open(self,h):
        self.assertTrue(h.rpc(dict(op='show',mode='analytics'))['ok'])
        h.settled(lambda s:s['managementUI']['visible'] and s['managementUI']['tab']=='dashboard')
        return self.read(h)
    def read(self,h):return h.probe('dashboard-read')
    def wait(self,h,predicate):
        end=time.monotonic()+5
        while time.monotonic()<end:
            v=self.read(h)
            if predicate(v):return v
            time.sleep(.02)
        self.fail(v)
    def click(self,h,index):h.probe('dashboard-control',index=index)
    def capture(self,h,name):
        v=h.probe('dashboard-screenshot')
        if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            folder=Path(folder);folder.mkdir(parents=True,exist_ok=True);shutil.copy2(h.profile/'native-dashboard-probe.png',folder/(name+'.png'));(folder/(name+'.json')).write_text(json.dumps(v,ensure_ascii=False,indent=2))
        return v
    def test_populated_paused_errors_confirmation_roles_values_and_local_key_loop(self):
        for theme in ['light','dark']:
            h=self.host(theme=theme,counts={'analytics.fixture':42});self.open(h)
            self.assertTrue(h.rpc(dict(op='execute',id='analytics.fixture'))['ok']);self.assertFalse(h.rpc(dict(op='execute',id='analytics.failure'))['ok'])
            v=self.wait(h,lambda v:len(v['surface']['commands'])==2 and len(v['surface']['errors'])==1)
            self.assertEqual(v['surface']['prior_successes'],42);self.assertEqual(v['surface']['uses'],44)
            self.assertEqual(v['buttons'][1]['role'],'AXCheckBox');self.assertEqual(v['buttons'][1]['axValue'],'1');self.assertTrue(all(b['role']=='AXValueIndicator' for b in v['bars']))
            for width,height in [(680,480),(980,680)]:h.probe('dashboard-size',width=width,height=height);self.capture(h,f'populated-{theme}-{width}-{height}')
            h.probe('dashboard-focus',index=0);self.assertEqual(self.read(h)['focused'],'Refresh');h.probe('dashboard-next-focus');self.assertEqual(self.read(h)['focused'],'Local analytics')
            self.click(h,0);self.wait(h,lambda v:not v['surface']['busy']);self.assertEqual(self.read(h)['focused'],'Local analytics')
            self.click(h,1);self.wait(h,lambda v:not v['surface']['busy'] and not v['surface']['enabled']);self.capture(h,f'paused-{theme}')
            self.click(h,3);v=self.wait(h,lambda v:v['surface']['confirming_reset']);self.assertEqual(v['focused'],'Cancel');self.capture(h,f'confirmation-{theme}')
            before=h.rpc(dict(op='analytics'))['result']['totalAttempts'];h.probe('key-event',code=36,flags=1<<20,chars='\r');h.settled();self.assertEqual(h.rpc(dict(op='analytics'))['result']['totalAttempts'],before);h.probe('key-event',code=53,flags=0,chars='');self.wait(h,lambda v:not v['surface']['confirming_reset'])
        h=self.host(initial={'corrupt':'fixture'});self.open(h);v=self.wait(h,lambda v:v['surface']['storage_error'] is not None);self.assertIn('could not be read',v['surface']['storage_error']);h.probe('dashboard-size',width=680,height=480);self.capture(h,'corrupt-data-error')

if __name__=='__main__':unittest.main(verbosity=2)
