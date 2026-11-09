"""Real source-driven AppKit loop + CLI/worker, prohibited offscreen profiles."""
import copy
import json
import subprocess
import time
import threading
import os
from pathlib import Path
import shutil
import unittest
import runtime_process as rt

TOOLS=rt.ROOT/'target/native-tools/debug/ruycast-native'
rt.HOST=TOOLS

class Bridge(rt.Host):
    def start(self):
        self.process=subprocess.Popen([str(TOOLS),'--native-hidden'],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
        end=time.monotonic()+8
        while time.monotonic()<end:
            if self.process.poll() is not None:
                out,err=self.process.communicate()
                raise AssertionError(out.decode()+err.decode())
            try:
                if self.rpc({'op':'status'},timeout=.3).get('ok'):return
            except (OSError,ValueError,rt.ClosedResponse):pass
            time.sleep(.02)
        raise AssertionError('Native bridge did not start')
    def settled(self,predicate=lambda value:True):
        return self.wait(lambda s: not s.get('configurationUpdating',False) and s.get('nativeUI',{}).get('revision',0)>0 and s['nativeUI']['revision']==s['nativeUI']['appliedRevision'] and predicate(s))
    def probe(self,action,**fields):
        value=self.rpc(dict(op='native.test',action=action,**fields))
        if not value.get('ok'):raise AssertionError(value)
        return value['result']

class NativeBridge(unittest.TestCase):
    def host(self):
        host=Bridge();self.addCleanup(host.close);host.settled();return host
    def capture(self,host,name):
        value=host.probe('screenshot')
        image=host.profile/'native-probe.png'
        self.assertTrue(image.read_bytes().startswith(b'\x89PNG\r\n\x1a\n'))
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            output=Path(evidence).parent/'bridge';output.mkdir(parents=True,exist_ok=True)
            shutil.copy2(image,output/(name+'.png'))
            (output/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value
    def test_real_main_loop_normal_quit_without_kill_fallback(self):
        host=self.host()
        status=host.rpc({'op':'status'})['result']
        self.assertFalse(status['headless'])
        self.assertFalse(host.probe('read')['windowShown'])
        self.assertTrue(host.rpc({'op':'quit'})['ok'])
        out,err=host.process.communicate(timeout=4)
        self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
        self.assertEqual(json.loads(out)['result']['state'],'stopped')
        self.assertFalse((host.profile/'control.sock').exists())
    def test_nested_native_run_loop_defers_source_reentry_without_losing_delivery(self):
        host=self.host();self.assertGreater(host.probe('source-reentry')['deferredDeliveries'],0);self.assertTrue(host.rpc({'op':'status'})['ok']);self.assertFalse(host.probe('read')['windowShown'])
    def test_failed_worker_start_releases_loop_and_profile(self):
        host=Bridge(start=False);self.addCleanup(host.close)
        host.config.write_text('broken')
        value=subprocess.run([str(TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=4)
        self.assertNotEqual(value.returncode,0)
        self.assertFalse((host.profile/'control.sock').exists())
        host.config.write_text(json.dumps(rt.DEFAULT));host.start();host.settled()
    def test_normal_binary_refuses_hidden_test_runtime(self):
        host=Bridge(start=False);self.addCleanup(host.close)
        normal=rt.ROOT/'target/debug/ruycast-native'
        value=subprocess.run([str(normal),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=4)
        self.assertNotEqual(value.returncode,0)
        self.assertFalse((host.profile/'control.sock').exists())
    def test_show_search_and_local_native_query_share_the_owner(self):
        host=self.host()
        self.assertTrue(host.rpc({'op':'show','query':'finder'})['ok'])
        status=host.settled(lambda s:s['nativeUI']['visible'])
        view=host.probe('read')
        self.assertFalse(view['windowShown'])
        self.assertEqual(view['input'],'finder')
        self.assertEqual(view['modelQuery'],'finder')
        host.probe('query',query='safari')
        host.settled(lambda s:s['nativeUI']['selectedId']=='app.safari')
        self.assertEqual(host.probe('read')['modelQuery'],'safari')
        cli=host.cli('search','safari')
        self.assertTrue(cli['ok'])
        self.assertEqual(cli['result'][0]['id'],'app.safari')
        host.probe('key',key='escape')
        host.settled(lambda s:not s['nativeUI']['visible'])
    def test_pending_native_query_is_not_clobbered_by_older_snapshot(self):
        host=self.host()
        host.rpc({'op':'show','query':'finder'});host.settled(lambda s:s['nativeUI']['visible'])
        reply=host.probe('stale-query',query='safari')
        self.assertEqual(reply['input'],'safari')
        host.settled(lambda s:s['nativeUI']['selectedId']=='app.safari')
        self.assertEqual(host.probe('read')['input'],'safari')
    def test_native_composition_survives_pending_snapshot_and_return(self):
        host=self.host()
        host.rpc({'op':'show','query':'finder'});host.settled(lambda s:s['nativeUI']['visible'])
        host.probe('ime')
        host.settled()
        self.assertFalse(host.probe('read')['windowShown'])
    def test_escape_hides_during_busy_owner_and_supersedes_older_show(self):
        host=self.host()
        host.rpc({'op':'show','query':'finder'});host.settled(lambda s:s['nativeUI']['visible'])
        host.probe('schedule-escape')
        result=[]
        thread=threading.Thread(target=lambda:result.append(host.rpc({'op':'native.delay'})))
        thread.start()
        time.sleep(.03)
        shown=host.rpc({'op':'show'})
        thread.join(timeout=2)
        self.assertFalse(thread.is_alive())
        self.assertTrue(result[0]['result']['localCloseDuringDelay'])
        self.assertFalse(shown['ok'],shown)
        self.assertIn('superseded',shown['error'])
        host.settled(lambda s:not s['nativeUI']['visible'])
        self.assertTrue(host.rpc({'op':'show'})['ok'])
        host.settled(lambda s:s['nativeUI']['visible'])
    def test_loaded_config_updates_one_live_native_catalog(self):
        host=self.host()
        host.rpc({'op':'show','query':'finder'});host.settled(lambda s:s['nativeUI']['visible'])
        config=copy.deepcopy(rt.DEFAULT)
        config['commands']=[c for c in config['commands'] if c['id']!='app.finder']
        host.replace(config)
        status=host.settled(lambda s:s['commands']==len(config['commands']))
        self.assertNotEqual(status['nativeUI']['selectedId'],'app.finder')
        self.assertFalse(host.probe('read')['windowShown'])
        host.config.write_text('broken PRIVATE-SENTINEL')
        host.settled(lambda s:s['configError'] is not None)
        self.assertNotIn('PRIVATE-SENTINEL',json.dumps(host.probe('read')))
        self.assertEqual(host.rpc({'op':'status'})['result']['commands'],len(config['commands']))
        self.capture(host,'invalid-config')
    def test_watched_theme_reaches_actual_native_appearance(self):
        host=self.host()
        host.rpc({'op':'show'});host.settled(lambda s:s['nativeUI']['visible'])
        config=copy.deepcopy(rt.DEFAULT)
        self.assertIsNone(host.probe('read')['explicitAppearance'])
        for theme in ['dark','light','graphite','system',None]:
            revision=host.rpc({'op':'status'})['result']['configurationRevision']
            if theme is None:config.pop('theme',None)
            else:config['theme']=theme
            host.replace(config)
            host.settled(lambda s:s['configurationRevision']>revision and s['theme']==(theme or 'system'))
            appearance=host.probe('read')
            if theme in ['system',None]:self.assertIsNone(appearance['explicitAppearance'])
            else:self.assertEqual('DarkAqua' in appearance['appearance'],theme in ['dark','graphite'])
            self.capture(host,'theme-'+(theme or 'unset'))
    def test_rapid_arrows_and_click_on_other_row_keep_intended_identity(self):
        host=self.host()
        config=copy.deepcopy(rt.DEFAULT)
        config['commands']=[dict(id=f'fixture.{i}',title=f'Synthetic note {i}',kind='url',target=f'https://example.invalid/{i}',aliases=[f'note{i}'],arguments=[]) for i in range(21)]
        host.replace(config);host.settled(lambda s:s['commands']==21)
        host.rpc({'op':'show','query':''});host.settled(lambda s:s['nativeUI']['visible'])
        expected=[c['id'] for c in host.cli('search','note')['result']]
        host.probe('burst',query='note',count=5)
        host.settled(lambda s:s['nativeUI']['selectedId']==expected[5])
        view=host.probe('read');self.assertEqual(view['selectedRow'],5)
        host.probe('click',row=1)
        status=host.settled(lambda s:s['nativeUI']['selectedId']==expected[1])
        self.assertEqual(host.probe('read')['selectedRow'],1)
        self.assertFalse(host.probe('read')['windowShown'])
        host.wait(lambda s:s['openRequestsRunning']==0);host.settled()
        self.assertIn('fixture boundary is not installed',json.dumps(host.probe('read')))
        self.capture(host,'missing-native-boundary')

    def test_offscreen_visibility_policy_records_actual_transition_with_passive_reads(self):
        host=self.host();before=host.probe('read');self.assertFalse(before['windowShown'])
        host.probe('fixture-force-visible')
        visible=host.probe('read');self.assertTrue(visible['windowShown']);self.assertFalse(visible['windowKey'] or visible['windowMain']);self.assertEqual(visible['hiddenVisibilityRepairs'],before['hiddenVisibilityRepairs'])
        # Repeated passive getters must not repair/mask the deliberate violation.
        self.assertTrue(host.probe('read')['windowShown'])
        self.assertTrue(host.rpc(dict(op='show',query='notes'))['ok']);host.settled()
        after=host.probe('read');self.assertFalse(after['windowShown'] or after['windowKey'] or after['windowMain']);self.assertEqual(after['hiddenVisibilityRepairs'],before['hiddenVisibilityRepairs']+1)
        repair=after['lastHiddenVisibilityRepair'];self.assertTrue(repair['visibleBefore']);self.assertFalse(repair['keyBefore'] or repair['mainBefore']);self.assertLess(repair['x'],-10000);self.assertLess(repair['y'],-10000);self.assertEqual(repair['phase'],'apply-view')
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            folder=Path(out).parent/'rich';folder.mkdir(exist_ok=True);(folder/'hidden-visibility-policy.json').write_text(json.dumps(dict(before=before,visible=visible,after=after),indent=2))

if __name__=='__main__':unittest.main(verbosity=2)
