"""Actual native host/owner registration bridge, complete owned Carbon SDK only."""
import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import shutil
import sys
import tempfile
import time
import unittest
import bridge_process as bridge
import actions_process as actions
import runtime_process as rt
import keyword_fixture

class NativeKeyboard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sdk = tempfile.TemporaryDirectory(prefix='native-keyboard-sdk-',dir=rt.ROOT/'target')
        cls.shim=Path(cls.sdk.name)/'keyboard.dylib'
        notification=keyword_fixture.notification_object(Path(cls.sdk.name))
        subprocess.run(['clang','-dynamiclib','-Wall','-Wextra','-Werror','-fobjc-arc','-fblocks','-framework','AppKit','-DRUYCAST_HID_HOST_FIXTURE',notification,str(rt.ROOT/'tests/keyword_shim.c'),str(rt.ROOT/'tests/keyboard_shim.c'),str(rt.ROOT/'tests/hid_shim.c'),str(rt.ROOT/'tests/workspace_shim.m'),'-o',str(cls.shim)],check=True,capture_output=True)
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            dest=Path(out)/'host-sdks';dest.mkdir(parents=True,exist_ok=True);artifact=dest/(cls.__name__+'.dylib');shutil.copy2(cls.shim,artifact)
            sources={str(rt.ROOT/'tests'/name):hashlib.sha256((rt.ROOT/'tests'/name).read_bytes()).hexdigest() for name in ['keyword_shim.c','keyword_notification_shim.m','keyword_fixture.py','keyboard_shim.c','hid_shim.c','workspace_shim.m']}
            (dest/(cls.__name__+'.json')).write_text(json.dumps(dict(sources=sources,artifact=str(artifact),sha256=hashlib.sha256(artifact.read_bytes()).hexdigest()),indent=2))
    @classmethod
    def tearDownClass(cls):cls.sdk.cleanup()
    def host(self,commands=None,start=True,optin=True,marker=True,theme=None):
        host=bridge.Bridge(start=False);self.addCleanup(host.close)
        if commands is not None or theme is not None:
            config=copy.deepcopy(rt.DEFAULT);config['commands']+=commands or []
            if theme is not None:config['theme']=theme
            host.replace(config)
        if marker:(host.base/'.ruycast-keyboard-fixture').write_text('own-keyboard-fixture-v1')
        host.env.update(DYLD_INSERT_LIBRARIES=str(self.shim),RUYCAST_KEYWORD_FIXTURE_HOME=str(host.base),RUYCAST_KEYBOARD_FIXTURE_HOME=str(host.base),RUYCAST_WORKSPACE_FIXTURE_HOME=str(host.base),RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD='trace')
        if optin:host.env['RUYCAST_NATIVE_TEST_KEYBOARD']='1'
        if start:
            host.start()
            host.settled(lambda s:s['keyboardGeneration']>0 if optin else not s['nativeKeyboardService'])
        return host
    def emit(self,host,code=49,modifiers=6144,**kwargs):return host.probe('keyboard-press',code=code,modifiers=modifiers,**kwargs)['statuses']
    def trace(self,host):return [json.loads(line) for line in (host.base/'keyboard-trace.jsonl').read_text().splitlines()]
    def wait_file(self,path):
        end=time.monotonic()+4
        while not path.exists():
            self.assertLess(time.monotonic(),end,'Own marker not observed: '+path.name);time.sleep(.002)
    def script(self,id='script.key',key='cmd+a',target='/usr/bin/true',args=None):
        return dict(id=id,title='Own keyboard command',kind='script',target=target,arguments=args or [],aliases=['keyfixture'],hotkey=key,timeout=5)
    def test_native_callback_toggles_launcher_and_two_queued_presses_preserve_toggle_order(self):
        host=self.host();self.assertFalse(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        self.assertEqual(self.emit(host),[0]);host.settled(lambda s:s['nativeUI']['visible'])
        self.assertFalse(host.probe('read')['windowShown'])
        self.emit(host);host.settled(lambda s:not s['nativeUI']['visible'])
        self.assertEqual(self.emit(host,count=2),[0,0]);host.settled(lambda s:not s['nativeUI']['visible'])
    def test_item_callback_uses_exact_command_not_selected_row_and_logs_hotkey_without_showing_palette(self):
        host=self.host(start=False);marker=host.base/'own-action'
        config=copy.deepcopy(rt.DEFAULT);config['commands'].append(self.script(target='/usr/bin/touch',args=[str(marker)]));host.replace(config);host.start();host.settled(lambda s:s['keyboardGeneration']>0)
        self.assertNotEqual(host.rpc({'op':'status'})['result']['nativeUI']['selectedId'],'script.key')
        self.assertEqual(self.emit(host,0,256),[0]);self.wait_file(marker)
        host.wait(lambda s:s['scriptsRunning']==0)
        self.assertEqual(host.rpc({'op':'usage'})['result'].get('script.key'),1)
        self.assertFalse(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        logs=host.rpc({'op':'logs'})['result'];self.assertTrue(any(x.get('source')=='hotkey' and x.get('commandID')=='script.key' for x in logs))
        analytics=host.rpc({'op':'analytics'})['result'];self.assertEqual(analytics['totalSuccesses'],1);self.assertEqual(next(iter(analytics['commands'].values()))['sources'],{'hotkey':1})

    def test_hotkey_snippet_failure_keeps_exact_analytics_source_before_missing_sdk_refusal(self):
        host=self.host(commands=[dict(id='snippet.key',title='Owned hotkey snippet',kind='snippet',target='Owned text',hotkey='cmd+a')])
        self.assertEqual(self.emit(host,0,256),[0]);end=time.monotonic()+3
        while True:
            document=host.rpc({'op':'analytics'})['result']
            if document['totalFailures']==1:break
            self.assertLess(time.monotonic(),end);time.sleep(.01)
        self.assertEqual(document['totalAttempts'],1);self.assertEqual(next(iter(document['commands'].values()))['sources'],{'hotkey':1});self.assertFalse(host.probe('read')['windowShown'])

    def test_reload_retires_old_ids_and_disabled_bindings(self):
        host=self.host(start=False);old=host.base/'old';new=host.base/'new'
        config=copy.deepcopy(rt.DEFAULT);config['commands'].append(self.script(target='/usr/bin/touch',args=[str(old)]));host.replace(config);host.start();host.settled(lambda s:s['keyboardGeneration']>0)
        prior=next(x['c'] for x in self.trace(host) if x['phase']=='register' and x['a']==0)
        config['commands'][-1].update(hotkey='cmd+b',arguments=[str(new)]);host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled()
        self.assertEqual(self.emit(host,id=prior),[-9874]);self.assertEqual(self.emit(host,0,256),[-9874]);self.assertFalse(old.exists());self.assertFalse(new.exists())
        self.assertEqual(self.emit(host,11,256),[0]);self.wait_file(new);host.wait(lambda s:s['scriptsRunning']==0)
        config['commands'][-1]['enabled']=False;host.replace(config);self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled();self.assertEqual(self.emit(host,11,256),[-9874])
        self.assertEqual(host.rpc({'op':'usage'})['result'].get('script.key'),1)
    def test_actual_recording_waits_for_physical_unregistration_and_restores_accepted_plan(self):
        host=self.host();host.rpc({'op':'show','query':'finder'});host.settled()
        actions.ActionsProcesses.hotkey(self,host)
        self.assertTrue(host.rpc({'op':'status'})['result']['recordingShortcut'])
        self.assertEqual(self.emit(host),[-9874]);self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        self.assertTrue(any(x['phase']=='unregister' for x in self.trace(host)))
        host.probe('cancel-editor');host.settled(lambda s:not s['recordingShortcut'] and not s['nativeUI']['editorOpen'])
        self.assertEqual(self.emit(host),[0]);host.settled(lambda s:not s['nativeUI']['visible'])
    def test_two_window_recorders_do_not_resume_on_other_window_cancel(self):
        host=self.host();host.rpc({'op':'show','query':'finder'});host.settled();actions.ActionsProcesses.hotkey(self,host)
        host.rpc({'op':'show','mode':'settings'});host.settled()
        self.assertTrue(host.rpc(dict(op='native.test',surface='settings',action='inline-hotkey',id='app.safari'))['ok'])
        host.wait(lambda s:s['managementUI']['editorOpen']);self.assertTrue(host.rpc(dict(op='native.test',surface='settings',action='read'))['result']['recording'])
        host.probe('cancel-editor');host.settled(lambda s:not s['nativeUI']['editorOpen'])
        self.assertTrue(host.rpc({'op':'status'})['result']['recordingShortcut']);self.assertEqual(self.emit(host),[-9874])
        host.rpc(dict(op='native.test',surface='settings',action='cancel-editor'));host.wait(lambda s:not s['recordingShortcut'])
        self.assertEqual(self.emit(host),[0])
    def test_headless_hidden_does_not_enable_keyboard_without_explicit_optin(self):
        host=self.host(optin=False);self.assertFalse((host.base/'keyboard-trace.jsonl').exists());self.assertFalse(host.rpc({'op':'status'})['result']['nativeKeyboardService'])
    def test_missing_owned_sdk_stamp_fails_before_any_global_target_or_handler(self):
        host=self.host(start=False,marker=False)
        run=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=4)
        self.assertNotEqual(run.returncode,0);self.assertIn('Complete keyboard fixture',run.stdout);self.assertFalse((host.base/'keyboard-trace.jsonl').exists())
    def test_positive_held_main_registration_keeps_owner_responsive_and_profile_locked_until_retirement(self):
        host=self.host();(host.base/'hold-register').touch();config=json.loads(host.config.read_bytes());config['launcherHotkey']='cmd+a';host.replace(config)
        self.wait_file(host.base/'keyboard-register-entered')
        try:
            at=time.monotonic();status=host.rpc({'op':'status'})['result'];self.assertTrue(status['configurationUpdating'])
            self.assertTrue(host.rpc({'op':'search','query':'finder'})['ok']);self.assertTrue(host.rpc({'op':'show','query':'safari'})['ok'])
            self.assertTrue(host.rpc({'op':'quit'})['ok']);self.assertLess(time.monotonic()-at,.6)
            run=subprocess.run([str(bridge.TOOLS),'--native-hidden'],env=host.env,capture_output=True,text=True,timeout=3)
            self.assertNotEqual(run.returncode,0);self.assertIn('already running',run.stdout)
            self.assertIsNone(host.process.poll())
        finally:(host.base/'keyboard-register-release').touch()
        out,err=host.process.communicate(timeout=5);self.assertEqual(host.process.returncode,0,out.decode()+err.decode())
        self.assertFalse((host.profile/'control.sock').exists())
        self.assertTrue(any(x['phase']=='unregister' for x in self.trace(host)))

    def test_launcher_from_hotkey_report_returns_to_commands_before_toggle_hides(self):
        code='import json;print(json.dumps(dict(ok=True,detail="Own keyboard report")))'
        host=self.host(commands=[self.script(target=sys.executable,args=['-c',code])])
        self.assertEqual(self.emit(host,0,256),[0])
        self.view(host,lambda v:v['scriptResult'] is not None)
        self.assertTrue(host.rpc({'op':'status'})['result']['nativeUI']['visible'])
        self.emit(host);host.settled(lambda s:s['nativeUI']['visible']);self.view(host,lambda v:v['scriptResult'] is None)
        self.emit(host);host.settled(lambda s:not s['nativeUI']['visible'])
    def test_failed_registration_does_not_publish_editor_save_and_old_routes_survive(self):
        host=self.host();host.rpc({'op':'show','query':'finder'});host.settled()
        before=host.config.read_bytes();generation=host.rpc({'op':'status'})['result']['keyboardGeneration']
        (host.base/'fail-code').write_text('11')
        actions.ActionsProcesses.hotkey(self,host);self.key(host,11,actions.CMD,'b')
        self.view(host,lambda v:v['saveEnabled'] and any('unavailable (-9878)' in t for t in v['screenText']))
        host.settled();self.assertEqual(host.config.read_bytes(),before)
        self.assertEqual(host.rpc({'op':'status'})['result']['keyboardGeneration'],generation)
        host.probe('cancel-editor');host.settled(lambda s:not s['recordingShortcut'])
        self.assertEqual(self.emit(host),[0]);host.settled(lambda s:not s['nativeUI']['visible'])
        (host.base/'fail-code').unlink()

    def test_pending_recorder_consumes_save_keys_and_cancels_during_positive_physical_hold(self):
        for theme,management in [('light',False),('dark',False),('light',True),('dark',True)]:
            with self.subTest(management=management,theme=theme):
                host=self.host(theme=theme);before=host.config.read_bytes()
                if management:
                    host.rpc({'op':'show','mode':'settings'});host.settled();surface='settings'
                else:
                    host.rpc({'op':'show','query':'finder'});host.settled();surface='palette'
                (host.base/'hold-unregister').touch()
                self.assertTrue(host.rpc(dict(op='native.test',surface=surface,action='schedule-pending-recorder',close=management))['ok'])
                if management:host.rpc(dict(op='native.test',surface='settings',action='inline-hotkey',id='app.safari'))
                else:self.key(host,43,actions.CMD|actions.SHIFT,',')
                self.wait_file(host.base/'keyboard-pending.json')
                proof=json.loads((host.base/'keyboard-pending.json').read_bytes())
                text=proof.pop('screenText');accessibility=proof.pop('accessibilityText')
                self.assertTrue(any('Preparing' in t for t in text));self.assertTrue(any('Preparing shortcut recording' in t for t in accessibility))
                if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
                    dest=Path(out).parent/'native-keyboard';dest.mkdir(parents=True,exist_ok=True);name='pending-%s-%s'%(theme,management)
                    shutil.copy2(host.base/'keyboard-pending.png',dest/(name+'.png'));(dest/(name+'.json')).write_text(json.dumps(dict(proof,screenText=text,accessibilityText=accessibility),ensure_ascii=False,indent=2))
                self.assertEqual(proof,dict(preparing=True,recording=False,consumed=[True,True],unchanged=True,cancelled=True,physicalHold=True))
                host.settled(lambda s:not s['recordingShortcut']);self.assertEqual(host.config.read_bytes(),before)
                self.assertIsNone(host.process.poll());host.close()

    def test_callback_during_commit_before_main_ack_is_deferred_until_exact_store_adoption(self):
        host=self.host(start=False);marker=host.base/'after-commit'
        cfg=copy.deepcopy(rt.DEFAULT);cfg['commands'].append(self.script(target='/usr/bin/touch',args=[str(marker)]));host.replace(cfg);host.start();host.settled(lambda s:s['keyboardGeneration']>0)
        prior=host.rpc({'op':'status'})['result']['keyboardGeneration']
        (host.base/'emit-latest-on-unregister').touch();(host.base/'hold-unregister').touch()
        cfg['commands'][-1]['hotkey']='cmd+b';host.replace(cfg);self.wait_file(host.base/'keyboard-unregister-entered')
        try:
            status=host.rpc({'op':'status'})['result'];self.assertTrue(status['configurationUpdating']);self.assertEqual(status['keyboardGeneration'],prior)
            self.assertFalse(marker.exists());self.assertEqual(host.rpc({'op':'usage'})['result'],{})
            self.assertTrue(any(x['phase']=='unregister-latest-callback' and x['b']==0 for x in self.trace(host)))
            (host.base/'emit-latest-on-unregister').unlink()
        finally:(host.base/'keyboard-unregister-release').touch()
        host.settled(lambda s:s['keyboardGeneration']>prior);self.wait_file(marker);host.wait(lambda s:s['scriptsRunning']==0)
        self.assertEqual(host.rpc({'op':'usage'})['result'].get('script.key'),1)

    def test_recorder_begin_reenters_registration_blocks_immediately_and_retires_new_handle(self):
        host=self.host();host.rpc({'op':'show','query':'finder'});host.settled();actions.ActionsProcesses.hotkey(self,host)
        host.probe('record');self.view(host,lambda v:not v['recording'] and not v['recordingPreparing']);host.settled(lambda s:not s['recordingShortcut'])
        (host.base/'hold-register').touch();(host.base/'pump-register').touch()
        host.probe('schedule-begin-recorder')
        cfg=json.loads(host.config.read_bytes());cfg['launcherHotkey']='cmd+a';host.replace(cfg)
        self.wait_file(host.base/'keyboard-nested-begin.json')
        proof=json.loads((host.base/'keyboard-nested-begin.json').read_bytes())
        self.assertEqual(proof,dict(preparing=True,recording=False,blockedCallback={'Ok':-9874},physicalHold=True))
        self.view(host,lambda v:v['recording']);host.settled(lambda s:not s['configurationUpdating'])
        trace=self.trace(host);new_id=next(x['c'] for x in trace if x['phase']=='register' and x['a']==0)
        self.assertTrue(any(x['phase']=='unregister' and x['c']==new_id for x in trace))
        self.assertEqual(self.emit(host,0,256),[-9874]);host.probe('cancel-editor');host.settled(lambda s:not s['recordingShortcut'])
        self.assertTrue(host.rpc({'op':'reload'})['ok']);host.settled();self.assertEqual(self.emit(host,0,256),[0])

    def capture(self,host,name):
        value=host.probe('screenshot');self.assertFalse(value['windowShown']);self.assertFalse(value['windowKey'])
        if out:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            dest=Path(out).parent/'native-keyboard';dest.mkdir(parents=True,exist_ok=True)
            shutil.copy2(host.profile/'native-probe.png',dest/(name+'.png'));(dest/(name+'.json')).write_text(json.dumps(value,ensure_ascii=False,indent=2))
        return value
    def test_current_native_copy_and_recorder_error_renders_in_both_themes(self):
        for theme in ['light','dark']:
            host=self.host(theme=theme);host.rpc({'op':'show','query':'finder'});host.settled();actions.ActionsProcesses.hotkey(self,host)
            self.capture(host,'recording-'+theme);(host.base/'fail-code').write_text('11');self.key(host,11,actions.CMD,'b')
            self.view(host,lambda v:v['saveEnabled'] and any('unavailable (-9878)' in t for t in v['screenText']))
            self.capture(host,'registration-error-'+theme);host.probe('cancel-editor');host.settled(lambda s:not s['recordingShortcut'])
            host.close()

    # Reuse actual native input helpers, never synthetic OS/global key injection.
    key=actions.ActionsProcesses.key
    view=actions.ActionsProcesses.view

if __name__=='__main__':unittest.main(verbosity=2)
