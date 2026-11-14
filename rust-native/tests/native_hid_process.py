"""Actual main/runtime HID integration, own Carbon+HID+workspace SDK/profile."""
import copy
import os
import json
from pathlib import Path
import subprocess
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
import native_keyboard_process as keyboard
import runtime_process as rt
import bridge_process as bridge

CMD=1<<20;CTRL=1<<18;SHIFT=1<<17;ALT=1<<19;HYPER=CMD|CTRL|SHIFT|ALT

class NativeHid(unittest.TestCase):
    @classmethod
    def setUpClass(cls):keyboard.NativeKeyboard.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):keyboard.NativeKeyboard.tearDownClass.__func__(cls)
    host=keyboard.NativeKeyboard.host
    wait_file=keyboard.NativeKeyboard.wait_file
    script=keyboard.NativeKeyboard.script
    view=keyboard.NativeKeyboard.view
    def hid(self,host,control='state',**values):return host.probe('keyboard-hid',control=control,**values)
    def configured(self,hotkey='right-cmd+a',hyper=True):
        host=self.host(start=False);marker=host.base/'hid-action'
        config=copy.deepcopy(rt.DEFAULT);config['hyperKeyEnabled']=hyper;config['separateCommandKeys']=True
        config['commands'].append(self.script(id='hid.action',key=hotkey,target='/usr/bin/touch',args=[str(marker)]))
        host.replace(config);host.start();host.wait(lambda s:s['hidGeneration']==s['keyboardGeneration'] and s['hidStatus']=='On')
        return host,marker,config
    def test_same_plan_reload_preserves_held_caps_and_uses_new_master_generation(self):
        host,_,config=self.configured()
        self.hid(host,'input',usage=57,down=True,time=0);before=self.hid(host)['state']
        previous=host.rpc({'op':'status'})['result']['keyboardGeneration']
        config['commands'][0]['title']='Own renamed Finder';host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
        host.wait(lambda s:s['keyboardGeneration']>previous and s['hidGeneration']==s['keyboardGeneration'])
        after=self.hid(host)['state'];self.assertEqual(after['managerCreates'],before['managerCreates'])
        self.assertEqual(self.hid(host,'event',type=10,key=2,flags=0)['event']['flags'],HYPER)
    def test_changed_disabled_binding_cannot_run_old_action(self):
        host,marker,config=self.configured(hyper=False)
        config['commands'][-1]['enabled']=False;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok'])
        host.wait(lambda s:s['hidStatus']=='Off')
        self.assertFalse(self.hid(host,'event',type=10,key=0,flags=CMD|16)['event']['consumed'])
        self.assertFalse(marker.exists())
    def test_two_actual_recorders_suppress_actions_keep_hyper_and_clear_old_tap_epoch(self):
        host,marker,_=self.configured('ctrl+left-control')
        self.hid(host,'input',usage=224,down=True,time=1)
        host.rpc({'op':'show','query':'finder'});host.wait(lambda s:s['nativeUI']['visible'])
        host.probe('key-event',code=43,flags=CMD|SHIFT,chars=',');host.wait(lambda s:s['recordingShortcut'])
        self.view(host,lambda v:v['recording'])
        host.rpc({'op':'show','mode':'settings'})
        host.wait(lambda s:s['managementUI']['visible'])
        host.rpc(dict(op='native.test',surface='settings',action='inline-hotkey',id='app.safari'))
        host.wait(lambda s:s['managementUI']['editorOpen'])
        deadline=time.monotonic()+4
        while not host.rpc(dict(op='native.test',surface='settings',action='read'))['result']['recording']:
            self.assertLess(time.monotonic(),deadline);time.sleep(.01)
        host.probe('cancel-editor');host.wait(lambda s:not s['nativeUI']['editorOpen'])
        recording=host.rpc({'op':'status'})['result']
        self.assertTrue(recording['recordingShortcut'])
        self.assertEqual(recording['hyper'],'On')
        self.assertEqual(recording['modifierTaps'],'Off')
        self.assertEqual(recording['sidedShortcuts'],'Off')
        self.hid(host,'input',usage=224,down=False,time=1.2)
        self.hid(host,'input',usage=224,down=True,time=1.3);self.hid(host,'input',usage=224,down=False,time=1.5)
        self.assertFalse(marker.exists())
        self.hid(host,'input',usage=57,down=True,time=2)
        self.assertEqual(self.hid(host,'event',type=10,key=2,flags=0)['event']['flags'],HYPER)
        self.hid(host,'input',usage=57,down=False,time=2.1)
        host.rpc(dict(op='native.test',surface='settings',action='cancel-editor'));host.wait(lambda s:not s['recordingShortcut'])
        self.hid(host,'input',usage=224,down=False,time=2.2)
        self.assertFalse(marker.exists())
        self.hid(host,'input',usage=224,down=True,time=3);self.hid(host,'input',usage=224,down=False,time=3.2);self.wait_file(marker)

    def test_eventless_recording_episode_invalidates_old_modifier_tap(self):
        host,marker,_=self.configured('ctrl+left-control',hyper=False)
        self.hid(host,'input',usage=224,down=True,time=1)
        host.rpc({'op':'show','query':'finder'});host.settled()
        host.probe('key-event',code=43,flags=CMD|SHIFT,chars=',');self.view(host,lambda v:v['recording'])
        host.probe('cancel-editor');host.wait(lambda s:not s['recordingShortcut'])
        self.hid(host,'input',usage=224,down=False,time=1.2)
        self.assertFalse(marker.exists())
        self.hid(host,'input',usage=224,down=True,time=2);self.hid(host,'input',usage=224,down=False,time=2.2);self.wait_file(marker)

    def test_sleep_and_inactive_session_require_separate_release_in_both_orders(self):
        for first,second,release_first,release_second in [
            ('sleep','resign','wake','active'),('resign','sleep','active','wake')]:
            host,marker,_=self.configured('ctrl+left-control')
            self.hid(host,'input',usage=224,down=True,time=1)
            for name in [first,second,first,second,release_first,'launch','terminate',release_first]:
                self.hid(host,'notification',name=name)
                status=host.rpc({'op':'status'})['result']
                self.assertEqual(status['hidStatus'],'Paused',name)
                state=self.hid(host)['state']
                self.assertFalse(state['manager'],name);self.assertEqual(state['caps'],0,name)
                self.assertFalse(marker.exists())
            self.hid(host,'notification',name=release_second);host.wait(lambda s:s['hidStatus']=='On')
            self.hid(host,'input',usage=224,down=False,time=1.2)
            self.assertFalse(marker.exists())
            self.hid(host,'input',usage=224,down=True,time=2)
            self.hid(host,'input',usage=224,down=False,time=2.2);self.wait_file(marker)

    def test_quit_hides_both_actual_offscreen_panels_and_recorders_before_held_sdk(self):
        for force in [True,False]:
            host,_,_=self.configured()
            host.rpc({'op':'show','query':'finder'});host.settled()
            host.probe('key-event',code=43,flags=CMD|SHIFT,chars=',');self.view(host,lambda v:v['recording'])
            host.rpc({'op':'show','mode':'settings'});host.wait(lambda s:s['managementUI']['visible'])
            host.rpc(dict(op='native.test',surface='settings',action='inline-hotkey',id='app.safari'))
            deadline=time.monotonic()+4
            while not host.rpc(dict(op='native.test',surface='settings',action='read'))['result']['recording']:
                self.assertLess(time.monotonic(),deadline);time.sleep(.01)
            armed=host.probe('keyboard-shutdown-witness',forceVisible=force)
            self.assertEqual(armed['panelsVisible'],[force,force]);self.assertEqual(armed['mainOrKey'],[False,False])
            if not force:self.assertEqual(armed['recordersActive'],[True,True])
            hold=host.base/'hid-hold-manager_close';hold.touch()
            try:
                self.assertTrue(host.rpc({'op':'quit'})['ok'])
                self.wait_file(host.base/'hid-entered-manager_close')
                self.assertIsNone(host.process.poll())
                proof=json.loads((host.base/'shutdown-sdk-witness.json').read_text())
                if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                    folder=Path(evidence);folder.mkdir(parents=True,exist_ok=True);(folder/f'quit-order-{force}.json').write_text(json.dumps(dict(armed=armed,atSdk=proof),indent=2))
                self.assertEqual(proof['pid'],host.process.pid);self.assertTrue(proof['mainThread'])
                self.assertFalse(proof['running'],proof);self.assertFalse(proof['recording'],proof)
                self.assertEqual(len(proof['panels']),2)
                for panel in proof['panels']:
                    self.assertFalse(panel['visible'],proof);self.assertFalse(panel['modelVisible'],proof)
                    self.assertFalse(panel['recording']);self.assertFalse(panel['preparing'])
                    self.assertTrue(panel['connectionStopped'])
            finally:
                hold.unlink(missing_ok=True)
                out,err=host.process.communicate(timeout=5);code=host.process.returncode;host.process=None
                self.assertEqual(code,0,out.decode()+err.decode())

    def test_status_is_not_on_while_main_is_positively_held_in_sdk_teardown(self):
        host,_,config=self.configured()
        hold=host.base/'hid-hold-manager_close';hold.touch()
        try:
            config['commands'][-1]['hotkey']='right-cmd+b';host.replace(config)
            with ThreadPoolExecutor(max_workers=1) as pool:
                pending=pool.submit(host.rpc,dict(op='reload'))
                try:
                    self.wait_file(host.base/'hid-entered-manager_close')
                    status=host.rpc({'op':'status'})['result']
                    self.assertNotEqual(status['hidStatus'],'On')
                    self.assertNotEqual(status['hyper'],'On')
                    self.assertNotEqual(status['sidedShortcuts'],'On')
                    self.assertTrue(status['hidRecoveryPending'])
                finally:hold.unlink(missing_ok=True)
                self.assertTrue(pending.result()['ok'])
            host.wait(lambda s:s['hidStatus']=='On' and s['hidGeneration']==s['keyboardGeneration'])
        finally:hold.unlink(missing_ok=True)

    def test_source_callback_failure_keeps_native_cleanup_and_profile_until_recovered(self):
        host,_,_=self.configured();self.hid(host,'input',usage=57,down=True,time=0)
        fault=host.base/'hid-fail-manager_close';fault.touch()
        try:
            try:host.probe('keyboard-source-failure')
            except (OSError,ValueError):pass
            time.sleep(.15);self.assertIsNone(host.process.poll())
            second=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3)
            self.assertNotEqual(second.returncode,0);self.assertIn('already running',second.stdout)
        finally:fault.unlink(missing_ok=True)
        expected_pid=host.process.pid
        out,err=host.process.communicate(timeout=5);code=host.process.returncode;host.process=None;self.assertNotEqual(code,0)
        self.assertIn('owned source callback failure',err.decode())
        proof=json.loads((host.base/'keyboard-retired.json').read_text());self.assertEqual(proof['pid'],expected_pid)
        witness=proof['state'];self.assertEqual(witness['allocated'],0);self.assertEqual(witness['caps'],0)

    def test_forwarded_hid_is_revoked_across_health_recovery_while_carbon_is_preserved(self):
        for blocked in [2,4,8]:
            host=self.host(start=False);hid_marker=host.base/'stale-hid';carbon_marker=host.base/'valid-carbon'
            config=copy.deepcopy(rt.DEFAULT);config['separateCommandKeys']=True
            hid=self.script(id='hid.action',key='right-cmd+a',target='/usr/bin/touch',args=[str(hid_marker)])
            carbon=self.script(id='carbon.action',key='cmd+b',target='/usr/bin/touch',args=[str(carbon_marker)]);carbon['aliases']=['carbon-own']
            config['commands'] += [hid,carbon];host.replace(config);host.start();host.wait(lambda s:s['hidStatus']=='On')
            steps=[dict(after=.02,control='event',type=10,key=0,flags=CMD|16),dict(after=.04,control='carbon',key=11,modifiers=256),
                   dict(after=.08,control='env',value=blocked),dict(after=.14,control='state'),dict(after=.18,control='env',value=0),
                   dict(after=.24,control='state'),dict(after=.28,control='release-owner')]
            host.probe('keyboard-schedule',steps=steps)
            with ThreadPoolExecutor(max_workers=1) as pool:
                pending=pool.submit(host.rpc,dict(op='native.delay',executionBarrier=True,holdOwner=True))
                try:self.wait_file(host.base/'hid-schedule-6.json')
                finally:(host.profile/'owner-delay-release').write_text('release')
                self.assertTrue(pending.result()['ok'])
            proofs=[json.loads((host.base/f'hid-schedule-{i}.json').read_text()) for i in range(7)]
            self.assertTrue(all(p['ownerEntered'] and p['ownerStillHeld'] for p in proofs))
            self.assertGreaterEqual(proofs[3]['forwarded'],1)
            self.assertNotEqual(proofs[3]['health']['status'],'On')
            self.assertEqual(proofs[5]['health']['status'],'On')
            self.assertGreater(proofs[5]['hookEpoch'],proofs[0]['hookEpoch'])
            self.wait_file(carbon_marker);host.wait(lambda s:s['scriptsRunning']==0)
            self.assertFalse(hid_marker.exists())
    def test_sleep_session_and_secure_recovery_report_health_without_destroying_master(self):
        host,marker,_=self.configured();generation=host.rpc({'op':'status'})['result']['keyboardGeneration']
        self.hid(host,'notification',name='sleep');host.wait(lambda s:s['hidStatus']=='Paused')
        self.assertFalse(self.hid(host,'event',type=10,key=0,flags=CMD|16)['event']['consumed'])
        self.hid(host,'notification',name='launch');self.assertEqual(host.rpc({'op':'status'})['result']['hidStatus'],'Paused')
        self.hid(host,'notification',name='wake');host.wait(lambda s:s['hidStatus']=='On')
        self.hid(host,'env',value=1);host.wait(lambda s:s['hidStatus']=='Paused during secure input')
        self.assertEqual(host.rpc({'op':'status'})['result']['keyboardGeneration'],generation)
        self.assertFalse(marker.exists());self.hid(host,'env',value=0);host.wait(lambda s:s['hidStatus']=='On')
    def test_permission_health_saves_plan_and_carbon_remains_available(self):
        host,_,config=self.configured(hyper=False)
        self.hid(host,'env',value=2);host.wait(lambda s:s['hidStatus'].startswith('Allow Accessibility'))
        config['commands'][0]['title']='Saved with unavailable HID';host.replace(config)
        self.assertTrue(host.rpc({'op':'reload'})['ok']);host.wait(lambda s:s['hidGeneration']==s['keyboardGeneration'])
        self.assertEqual(json.loads(host.config.read_bytes())['commands'][0]['title'],'Saved with unavailable HID')
        self.assertEqual(host.probe('keyboard-press',code=49,modifiers=6144)['statuses'],[0])
        host.wait(lambda s:s['nativeUI']['visible'])
        self.hid(host,'env',value=0);host.wait(lambda s:s['hidStatus']=='On')
    def test_refresh_timer_recovers_file_fault_without_another_input_or_notification(self):
        host,_,_=self.configured();(host.base/'hid-fail-tap_create').touch()
        self.hid(host,'event',type=0xffffffff,key=0,flags=0)
        host.wait(lambda s:s['hidStatus']!='On')
        (host.base/'hid-fail-tap_create').unlink()
        host.wait(lambda s:s['hidStatus']=='On')
    def test_quit_retains_profile_and_main_recovery_after_worker_stop(self):
        for fault in ['hid-fail-manager_close','hid-fail-invalidate','caps-fail-set:false']:
            host,_,_=self.configured();self.hid(host,'input',usage=57,down=True,time=0)
            path=host.base/fault;path.touch()
            try:
                self.assertTrue(host.rpc({'op':'quit'})['ok'])
                time.sleep(.15);self.assertIsNone(host.process.poll(),fault)
                second=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3)
                self.assertNotEqual(second.returncode,0);self.assertIn('already running',second.stdout)
            finally:path.unlink(missing_ok=True)
            expected_pid=host.process.pid
            out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
            proof=json.loads((host.base/'keyboard-retired.json').read_text());self.assertEqual(proof['pid'],expected_pid)
            witness=proof['state'];self.assertEqual(witness['allocated'],0);self.assertEqual(witness['caps'],0)

if __name__=='__main__':unittest.main(verbosity=2)
