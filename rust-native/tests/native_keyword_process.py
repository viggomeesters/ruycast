"""Actual Rust owner/master generation + keyword service, entirely own SDK."""
import copy,json,os,subprocess,time,unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import native_keyboard_process as keyboard
import keyword_capture_process as capture
import bridge_process as bridge
import actions_process as actions

class NativeKeyword(unittest.TestCase):
    @classmethod
    def setUpClass(cls):keyboard.NativeKeyboard.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):keyboard.NativeKeyboard.tearDownClass.__func__(cls)
    key=actions.ActionsProcesses.key
    host=keyboard.NativeKeyboard.host;wait_file=keyboard.NativeKeyboard.wait_file;view=keyboard.NativeKeyboard.view
    def kw(self,host,control='state',**values):return host.probe('keyboard-keyword',control=control,**values)
    def type(self,host,text):return [self.kw(host,'event',text=c)['consumed'] for c in text]
    def configured(self,**values):
        host=self.host(start=False);c=capture.config();c.update(values);host.replace(c);host.start();host.wait(lambda s:s['keywordGeneration']==s['keyboardGeneration'] and s['snippetExpansion']=='On');return host,c
    def wait_generation(self,host,old):return host.wait(lambda s:s['keyboardGeneration']>old and s['keywordGeneration']==s['keyboardGeneration'] and not s['configurationUpdating'])
    def configured_rich(self,text='A{cursor}BC',mode='html'):
        host=self.host(start=False);c=capture.config(text,keyword='x',snippetOutput=mode);host.replace(c);host.start();host.wait(lambda s:s['keywordGeneration']==s['keyboardGeneration'] and s['snippetExpansion']=='On');return host,c
    def retire(self,host):
        self.assertTrue(host.rpc({'op':'quit'})['ok']);out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());return json.loads((host.base/'keyword-retired.json').read_text())
    def test_quit_during_rich_clipboard_or_paste_retains_profile_until_cleanup(self):
        for phase in ['board_clear','board_write','post:down']:
            with self.subTest(phase=phase):
                host,_=self.configured_rich();(host.base/('hold-keyword-'+phase)).touch()
                with ThreadPoolExecutor(1) as pool:
                    pending=pool.submit(self.kw,host,'event',text='x');self.wait_file(host.base/('keyword-'+phase+'-entered'))
                    try:
                        self.assertTrue(host.rpc({'op':'quit'})['ok']);second=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3);self.assertNotEqual(second.returncode,0);self.assertIn('already running',second.stdout);self.assertIsNone(host.process.poll())
                    finally:(host.base/('keyword-'+phase+'-release')).touch()
                    try:pending.result(timeout=5)
                    except AssertionError as error:self.assertIn('closed without response',str(error))
                out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());proof=json.loads((host.base/'keyword-retired.json').read_text());self.assertFalse(proof['ownActive']);self.assertEqual(proof['state']['allocated'],0);output=proof['state']['output']
                if phase=='post:down':self.assertEqual(len(output),2);self.assertEqual(output[0]['key'],9);self.assertEqual(output[-1]['units'],[]);self.assertEqual(output[-1]['flags'],0);self.assertFalse(output[-1]['down'])
                else:self.assertEqual(output,[])
    def reload(self,host,c):
        before=host.rpc({'op':'status'})['result']['keyboardGeneration'];host.replace(c);reply=host.rpc({'op':'reload'});self.assertTrue(reply['ok'] or 'being updated' in reply.get('error',''),reply);return self.wait_generation(host,before)
    def reset(self,host):self.kw(host,'event',text='');self.kw(host,'clear')
    def test_pending_store_confirmation_keeps_observation_without_old_effects(self):
        host=self.host(start=False);c=capture.config();host.replace(c);host.env['RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER']=str(host.base)
        host.start();host.wait(lambda s:s['snippetExpansion']=='On');self.kw(host,'activation-listen');self.type(host,'ab');before=host.rpc({'op':'status'})['result']['keyboardGeneration']
        marker=host.base/'config-adoption-entered';release=host.base/'config-adoption-release';(host.base/'hold-config-adoption').touch();self.assertFalse(marker.exists())
        changed=copy.deepcopy(c);changed['launcherHotkey']='cmd+b';host.replace(changed)
        try:
            self.wait_file(marker);status=host.wait(lambda s:s['configurationUpdating'] and s['keywordGeneration']==0 and s['snippetExpansion']=='Configuration pending',timeout=2);state=self.kw(host);self.assertTrue(state['tapValid']);self.assertTrue(state['tapEnabled']);self.assertEqual(state['tapCreates'],1)
            host.probe('keyboard-hid',control='activation-burst',values=['typed','missing','typed'])
            events=self.kw(host,'activation-state')['events'];self.assertEqual([v['typed'] for v in events],[True,False,True]);self.assertEqual([v['sequence'] for v in events],list(range(events[0]['sequence'],events[0]['sequence']+3)))
            status=host.rpc({'op':'status'})['result'];self.assertTrue(status['configurationUpdating']);self.assertEqual(status['keywordGeneration'],0)
            self.assertFalse(any(self.type(host,'abc')));self.assertEqual(self.kw(host)['posts'],0)
        finally:release.touch()
        self.wait_generation(host,before);self.assertFalse(self.type(host,'c')[-1]);self.reset(host);self.assertTrue(self.type(host,'abc')[-1]);self.assertEqual(self.kw(host)['tapCreates'],1)
    def test_adoption_barrier_requires_exact_owned_fixture_home(self):
        host=self.host(start=False);host.replace(capture.config());foreign=host.base/'unmarked';foreign.mkdir();host.env['RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER']=str(foreign);before=host.config.read_bytes()
        with self.assertRaisesRegex(AssertionError,'Own config adoption fixture required'):host.start()
        self.assertEqual(host.process.poll(),1);host.process.communicate(timeout=3);host.process=None;self.assertEqual(host.config.read_bytes(),before)
    def test_actual_two_recorders_suppress_until_both_cancel(self):
        host,_=self.configured();self.type(host,'ab');host.rpc({'op':'show','query':''});host.settled();actions.ActionsProcesses.hotkey(self,host)
        self.assertEqual(host.rpc({'op':'status'})['result']['snippetExpansion'],'Off');host.rpc({'op':'show','mode':'settings'});host.settled();host.rpc(dict(op='native.test',surface='settings',action='inline-hotkey',id='own.snippet'));host.wait(lambda s:s['managementUI']['editorOpen'])
        deadline=time.monotonic()+4
        while not host.rpc(dict(op='native.test',surface='settings',action='read'))['result']['recording']:
            self.assertLess(time.monotonic(),deadline);time.sleep(.005)
        host.probe('cancel-editor');host.wait(lambda s:not s['nativeUI']['editorOpen']);self.assertTrue(host.rpc({'op':'status'})['result']['recordingShortcut']);self.assertFalse(self.type(host,'c')[-1]);self.assertEqual(self.kw(host)['posts'],0)
        host.rpc(dict(op='native.test',surface='settings',action='cancel-editor'));host.wait(lambda s:not s['recordingShortcut']);self.assertEqual(host.probe('keyboard-press',code=49,modifiers=6144)['statuses'],[0]);host.wait(lambda s:not s['nativeUI']['visible']);self.assertFalse(self.type(host,'c')[-1]);self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
    def test_quit_during_entered_effect_keeps_profile_until_keyup_and_retirement(self):
        host,_=self.configured();self.type(host,'ab');(host.base/'hold-keyword-post:down').touch()
        with ThreadPoolExecutor(1) as pool:
            pending=pool.submit(self.kw,host,'event',text='c');self.wait_file(host.base/'keyword-post:down-entered')
            try:
                self.assertTrue(host.rpc({'op':'quit'})['ok']);second=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3);self.assertNotEqual(second.returncode,0);self.assertIn('already running',second.stdout);self.assertIsNone(host.process.poll())
            finally:(host.base/'keyword-post:down-release').touch()
            try:pending.result(timeout=5)
            except AssertionError as error:self.assertIn('closed without response',str(error))
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());proof=json.loads((host.base/'keyword-retired.json').read_text())['state'];self.assertEqual(proof['allocated'],0);self.assertEqual(proof['apps'],0);self.assertEqual(len(proof['output']),2);self.assertFalse(proof['output'][-1]['down']);self.assertEqual(proof['output'][-1]['units'],[])
if __name__=='__main__':unittest.main(verbosity=2)
