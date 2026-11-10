"""Actual Rust owner/master generation + keyword service, entirely own SDK."""
import copy,json,os,subprocess,time,unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import native_keyboard_process as keyboard
import keyword_capture_process as capture
import bridge_process as bridge
import actions_process as actions

class NativeKeyword(unittest.TestCase):
    def test_each_typed_activation_reaches_keyword_owner_before_refresh_coalescing(self):
        host,_=self.configured();self.kw(host,'activation-listen');self.type(host,'ab')
        host.probe('keyboard-hid',control='activation-burst',values=['typed','missing','malformed','typed'])
        events=self.kw(host,'activation-state')['events']
        self.assertEqual(len(events),4)
        self.assertEqual([v['typed'] for v in events],[True,False,False,True])
        self.assertEqual([v['ownInstance'] for v in events],[True,False,False,True])
        self.assertEqual([v['sequence'] for v in events],list(range(events[0]['sequence'],events[0]['sequence']+4)))
        self.assertFalse(self.type(host,'c')[-1]);self.assertEqual(self.kw(host)['posts'],0)
        self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
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
    def test_rich_native_host_uses_shared_own_write_and_real_cursor_timer(self):
        for mode in ['html','teams']:
            with self.subTest(mode=mode):
                host,_=self.configured_rich(mode=mode);self.assertEqual(self.type(host,'x'),[True])
                # No status/probe RPC pumps the host: the actual common-mode
                # timer must wake it and issue the pending cursor itself.
                self.wait_file(host.base/'keyword-cursor-entered')
                state=self.kw(host);self.assertEqual(bytes(state['pasteText']).decode(),'ABC');self.assertEqual(state['pasteIssues'],1);self.assertEqual(sum(e['key']==123 and e['down'] for e in state['output']),2);self.assertFalse(host.probe('read')['windowShown'])
                proof=self.retire(host);self.assertEqual(proof['ownChange'],state['boardCount']);self.assertFalse(proof['ownActive']);self.assertEqual(proof['state']['allocated'],0)
    def test_new_native_input_cancels_rich_cursor_before_timer(self):
        host,_=self.configured_rich();self.assertEqual(self.type(host,'x'),[True]);self.kw(host,'event',text='z');time.sleep(.15);state=self.kw(host);self.assertEqual(state['pasteIssues'],1);self.assertFalse((host.base/'keyword-cursor-entered').exists());self.assertEqual(sum(e['key']==123 and e['down'] for e in state['output']),0)
    def test_native_partial_rich_write_is_suppressed_but_trigger_passes(self):
        host,_=self.configured_rich();(host.base/'keyword-fail-board_partial').touch();self.assertEqual(self.type(host,'x'),[False]);state=self.kw(host);self.assertEqual(state['posts'],0);self.assertEqual(bytes(state['boardText']).decode(),'ABC');self.assertIsNone(state['boardHTML']);proof=self.retire(host);self.assertEqual(proof['ownChange'],state['boardCount']);self.assertFalse(proof['ownActive'])
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
    def test_actual_host_adopts_prepared_catalog_and_emits_without_showing_window(self):
        host,_=self.configured();self.assertEqual(self.type(host,'abc'),[False,False,True]);s=self.kw(host);self.assertEqual(capture.KeywordCapture.inserted(self,s),'HELLO');self.assertEqual(s['valueReads'],0);self.assertFalse(host.probe('read')['windowShown']);self.assertFalse(host.rpc({'op':'status'})['result']['snippetExpansionComplete'])
    def test_reload_uses_confirmed_new_catalog_and_disable_retires_filter(self):
        host,c=self.configured();self.type(host,'ab');c['commands'][0]['target']='NEW';self.reload(host,c);self.assertEqual(self.type(host,'c'),[False]);self.reset(host);self.assertTrue(self.type(host,'abc')[-1]);self.assertEqual(capture.KeywordCapture.inserted(self,self.kw(host)),'NEW')
        c['snippetExpansionEnabled']=False;self.reload(host,c);host.wait(lambda s:s['snippetExpansion']=='Off');self.assertFalse(self.kw(host)['tapValid']);self.assertFalse(self.type(host,'abc')[-1])
    def test_failed_registration_keeps_last_good_catalog_after_owner_reconciliation(self):
        host,c=self.configured();before=host.rpc({'op':'status'})['result']['keyboardGeneration'];(host.base/'fail-code').write_text('11');c['launcherHotkey']='cmd+b';host.replace(c);reply=host.rpc({'op':'reload'});host.wait(lambda s:not s['configurationUpdating'] and s['keyboardGeneration']==before);self.assertEqual(host.rpc({'op':'status'})['result']['keywordGeneration'],before);self.assertTrue(self.type(host,'abc')[-1]);(host.base/'fail-code').unlink()
    def test_actual_two_recorders_suppress_until_both_cancel(self):
        host,_=self.configured();self.type(host,'ab');host.rpc({'op':'show','query':''});host.settled();actions.ActionsProcesses.hotkey(self,host)
        self.assertEqual(host.rpc({'op':'status'})['result']['snippetExpansion'],'Off');host.rpc({'op':'show','mode':'settings'});host.settled();host.rpc(dict(op='native.test',surface='settings',action='inline-hotkey',id='own.snippet'));host.wait(lambda s:s['managementUI']['editorOpen'])
        deadline=time.monotonic()+4
        while not host.rpc(dict(op='native.test',surface='settings',action='read'))['result']['recording']:
            self.assertLess(time.monotonic(),deadline);time.sleep(.005)
        host.probe('cancel-editor');host.wait(lambda s:not s['nativeUI']['editorOpen']);self.assertTrue(host.rpc({'op':'status'})['result']['recordingShortcut']);self.assertFalse(self.type(host,'c')[-1]);self.assertEqual(self.kw(host)['posts'],0)
        host.rpc(dict(op='native.test',surface='settings',action='cancel-editor'));host.wait(lambda s:not s['recordingShortcut']);self.assertEqual(host.probe('keyboard-press',code=49,modifiers=6144)['statuses'],[0]);host.wait(lambda s:not s['nativeUI']['visible']);self.assertFalse(self.type(host,'c')[-1]);self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
    def test_actual_hid_hyper_episode_between_keyword_events_clears_prefix(self):
        host,_=self.configured(hyperKeyEnabled=True);host.wait(lambda s:s['hyper']=='On');self.type(host,'ab')
        for down,t in [(True,1),(False,1.1)]:host.probe('keyboard-hid',control='input',usage=57,down=down,time=t)
        self.assertFalse(self.type(host,'c')[-1]);self.assertEqual(self.kw(host)['posts'],0);self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
    def test_external_activation_roundtrip_invalidates_prefix_even_same_retained_app(self):
        host,_=self.configured();self.type(host,'ab')
        for _ in range(2):host.probe('keyboard-hid',control='notification',name='activate')
        self.assertFalse(self.type(host,'c')[-1]);self.assertEqual(self.kw(host)['posts'],0);self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
    def test_sleep_session_interleaving_and_resumption(self):
        host,_=self.configured();self.type(host,'ab')
        for name in ['sleep','resign','wake']:host.probe('keyboard-hid',control='notification',name=name)
        host.wait(lambda s:s['snippetExpansion']=='Paused');self.assertFalse(self.type(host,'c')[-1]);host.probe('keyboard-hid',control='notification',name='active');host.wait(lambda s:s['snippetExpansion']=='On');self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
    def test_native_secure_exclusion_and_permission_changes_do_not_expand(self):
        host,c=self.configured()
        for flags in [8,16,4096]:
            self.kw(host,'environment',flags=flags);self.assertFalse(self.type(host,'abc')[-1]);self.assertEqual(self.kw(host)['posts'],0)
        self.kw(host,'environment');c['clipboard']['excludedApps']=['own.keyword.target'];self.reload(host,c);self.assertFalse(self.type(host,'abc')[-1]);self.assertEqual(self.kw(host)['posts'],0)
    def test_timeout_callback_recovers_on_actual_main_wake_and_drops_prefix(self):
        host,_=self.configured();self.type(host,'ab');self.kw(host,'event',type=0xffffffff);host.wait(lambda s:s['snippetExpansion']=='On');self.assertFalse(self.type(host,'c')[-1]);self.reset(host);self.assertTrue(self.type(host,'abc')[-1])
    def test_timer_recovers_failed_keyword_setup_without_new_input(self):
        host,_=self.configured();(host.base/'keyword-fail-tap_create').touch();self.kw(host,'event',type=0xffffffff);host.wait(lambda s:s['snippetExpansion']!='On');(host.base/'keyword-fail-tap_create').unlink();host.wait(lambda s:s['snippetExpansion']=='On');self.assertTrue(self.type(host,'abc')[-1])
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
    def test_quit_retries_failed_invalidation_and_retains_instance_lock(self):
        host,_=self.configured();fault=host.base/'keyword-fail-invalidate';fault.touch()
        try:
            self.assertTrue(host.rpc({'op':'quit'})['ok']);time.sleep(.1);self.assertIsNone(host.process.poll());second=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3);self.assertNotEqual(second.returncode,0);self.assertIn('already running',second.stdout)
        finally:fault.unlink(missing_ok=True)
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode());proof=json.loads((host.base/'keyword-retired.json').read_text())['state'];self.assertEqual(proof['allocated'],0);self.assertFalse(proof['tapValid'])
if __name__=='__main__':unittest.main(verbosity=2)
