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
    def test_management_routing_controls_custom_retention_and_reset(self):
        h=self.host();v=self.open(h);self.assertTrue(v['visible']);self.assertEqual(len(v['surface']['hours']),24)
        self.assertIn('Dashboard',v['text']);self.assertFalse(h.probe('read')['windowShown'])
        self.click(h,1);self.wait(h,lambda v:not v['surface']['busy'] and not v['surface']['enabled'])
        self.assertFalse(h.rpc(dict(op='analytics'))['result']['enabled'])
        self.assertTrue(h.rpc(dict(op='analytics.settings',retentionDays=14))['ok'])
        self.assertTrue(h.rpc(dict(op='execute',id='analytics.fixture'))['ok'])
        v=self.wait(h,lambda v:v['surface']['retention_days']==14)
        self.assertIn('14 days',v['text'])
        h.probe('dashboard-retention',days=7);self.wait(h,lambda v:not v['surface']['busy'] and v['surface']['retention_days']==7)
        before=h.config.read_bytes();self.click(h,3);self.wait(h,lambda v:v['surface']['confirming_reset'])
        self.click(h,5);self.wait(h,lambda v:not v['surface']['confirming_reset'])
        self.click(h,3);self.wait(h,lambda v:v['surface']['confirming_reset']);self.click(h,4)
        self.wait(h,lambda v:not v['surface']['busy'] and not v['surface']['confirming_reset'])
        self.assertEqual(h.config.read_bytes(),before)
    def test_standard_builtin_cli_native_sources_usage_logs_and_override(self):
        from library_process import Palette
        h=self.host();p=Palette(h)
        rows=h.rpc(dict(op='search',query='dashboard'))['result'];self.assertEqual([r['id'] for r in rows],['builtin.dashboard'])
        self.assertTrue(h.rpc(dict(op='execute',id='analytics',dryRun=True))['dryRun'])
        self.assertTrue(h.rpc(dict(op='execute',id='analytics'))['ok']);h.settled(lambda s:s['managementUI']['tab']=='dashboard')
        d=h.rpc(dict(op='analytics'))['result'];self.assertEqual(d['totalAttempts'],1);self.assertEqual(d['totalSuccesses'],1)
        self.assertEqual(h.rpc(dict(op='usage'))['result']['builtin.dashboard'],1)
        logs=h.rpc(dict(op='logs',limit=100))['result'];self.assertTrue(any(r['commandID']=='builtin.dashboard' and r['source']=='cli' and r['phase']=='command.finish' for r in logs))
        h.rpc(dict(op='show',query='dashboard'));h.wait(lambda s:s['nativeUI']['visible']);p.probe('key-event',code=36,flags=0,chars='\r');h.settled(lambda s:s['managementUI']['tab']=='dashboard' and not s['nativeUI']['visible'])
        self.assertEqual(h.rpc(dict(op='usage'))['result']['builtin.dashboard'],2)
        d=h.rpc(dict(op='analytics'))['result'];self.assertEqual(d['totalAttempts'],2);self.assertEqual(d['totalSuccesses'],2)
        c=json.loads(h.config.read_text());c['commands'].append(dict(id='own.dashboard',title='Own dashboard',kind='builtin',target='analytics',aliases=['myactivity'],enabled=False));h.replace(c);self.assertTrue(h.rpc(dict(op='reload'))['ok']);h.settled()
        self.assertEqual(h.rpc(dict(op='search',query='dashboard'))['result'],[])
        self.assertFalse(h.rpc(dict(op='execute',id='myactivity'))['ok'])
    def test_builtin_input_validation_and_native_prompt_do_not_log_private_values(self):
        from library_process import Palette
        h=self.host();p=Palette(h);c=json.loads(h.config.read_text());c['commands'].append(dict(id='own.prompted-dashboard',title='Prompted analytics',kind='builtin',target='analytics',aliases=['promptactivity'],inputPrompt='Reason'));h.replace(c);self.assertTrue(h.rpc(dict(op='reload'))['ok']);h.settled()
        self.assertTrue(h.rpc(dict(op='execute',id='promptactivity',dryRun=True))['dryRun'])
        for values in [{},dict(input=' \n\t'),dict(input=2)]:self.assertFalse(h.rpc(dict(op='execute',id='promptactivity',values=values))['ok'])
        self.assertEqual(h.rpc(dict(op='analytics'))['result']['totalAttempts'],0)
        h.rpc(dict(op='show',query='promptactivity'));h.wait(lambda s:s['nativeUI']['visible']);p.probe('key-event',code=36,flags=0,chars='\r');h.wait(lambda s:s['nativeUI']['editorOpen']);self.assertTrue(p.probe('read')['runInputInline']);self.assertEqual(h.rpc(dict(op='analytics'))['result']['totalAttempts'],0)
        p.probe('run-inline-type',text='PRIVATE dashboard reason');p.probe('key-event',code=36,flags=0,chars='\r');h.settled(lambda s:s['managementUI']['tab']=='dashboard' and not s['nativeUI']['visible']);self.assertEqual(h.rpc(dict(op='analytics'))['result']['totalAttempts'],1);self.assertNotIn('PRIVATE',json.dumps(h.rpc(dict(op='logs'))))
    def test_repeated_confirmation_retires_old_queued_reset_callback(self):
        h=self.host();self.open(h);self.assertTrue(h.rpc(dict(op='execute',id='analytics.fixture'))['ok'])
        self.click(h,3);first=self.wait(h,lambda v:v['surface']['confirming_reset']);h.probe('dashboard-retain',index=4);self.click(h,5)
        self.wait(h,lambda v:not v['surface']['confirming_reset']);self.click(h,3);second=self.wait(h,lambda v:v['surface']['confirming_reset']);self.assertNotEqual(first['surface']['confirmation_token'],second['surface']['confirmation_token'])
        before=h.rpc(dict(op='analytics'))['result']['totalAttempts'];h.probe('dashboard-stale');self.assertEqual(h.rpc(dict(op='analytics'))['result']['totalAttempts'],before);self.assertTrue(self.read(h)['surface']['confirming_reset'])
        self.click(h,4);self.wait(h,lambda v:not v['surface']['busy'] and not v['surface']['confirming_reset']);self.assertEqual(h.rpc(dict(op='analytics'))['result']['totalAttempts'],0)
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
    def test_actual_dashboard_renders_themes_widths_and_preserves_scroll(self):
        for theme in ['light','dark']:
            h=self.host(theme=theme);self.open(h)
            for width in [680,980]:
                h.probe('dashboard-size',width=width,height=480)
                v=h.probe('dashboard-screenshot');self.assertEqual(v['width'],width)
                self.assertIn('Retention',v['accessibility'])
                self.assertTrue(any('uses' in text or text=='Not recorded' for text in v['accessibility']))
                if folder:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                    out=Path(folder).parent/'dashboard';out.mkdir(parents=True,exist_ok=True)
                    shutil.copy2(h.profile/'native-dashboard-probe.png',out/f'{theme}-{width}.png')
                    (out/f'{theme}-{width}.json').write_text(json.dumps(v,ensure_ascii=False,indent=2))
                h.probe('dashboard-scroll',y=200);before=self.read(h)['scrollY'];self.click(h,0)
                self.wait(h,lambda v:not v['surface']['busy']);self.assertEqual(self.read(h)['scrollY'],before)

if __name__=='__main__':unittest.main(verbosity=2)
