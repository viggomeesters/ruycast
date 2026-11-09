"""General's real RecordButtons arbitrate the existing complete synthetic SDK."""
import json,time,unittest
import native_keyboard_process as nk
import actions_process as actions
class GeneralKeyboard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):nk.NativeKeyboard.setUpClass.__func__(cls)
    @classmethod
    def tearDownClass(cls):cls.sdk.cleanup()
    def host(self):return nk.NativeKeyboard.host(self)
    def sp(self,h,action,**fields):
        v=h.rpc(dict(op='native.test',surface='settings',action=action,**fields));self.assertTrue(v['ok'],v);return v['result']
    def wait(self,h,predicate):
        end=time.monotonic()+4
        while time.monotonic()<end:
            v=self.sp(h,'settings-read')
            if predicate(v):return v
            time.sleep(.01)
        self.fail(v)
    def general(self,h):h.rpc(dict(op='show',mode='settings'));h.settled();self.sp(h,'library-tab',tab='general');self.wait(h,lambda v:v['visible'])
    def test_general_recorder_uses_shared_lease_and_cannot_resume_peer_recorder(self):
        h=self.host();h.rpc(dict(op='show',query='finder'));h.settled();actions.ActionsProcesses.hotkey(self,h);self.assertTrue(h.rpc(dict(op='status'))['result']['recordingShortcut'])
        self.general(h);self.sp(h,'settings-record',index=0);self.wait(h,lambda v:v['recording'][0]);self.assertEqual(nk.NativeKeyboard.emit(self,h),[-9874])
        self.sp(h,'key-event',code=53,flags=0,chars='');self.wait(h,lambda v:not any(v['recording']));self.assertTrue(h.rpc(dict(op='status'))['result']['recordingShortcut']);self.assertEqual(nk.NativeKeyboard.emit(self,h),[-9874])
        h.probe('cancel-editor');h.settled(lambda s:not s['recordingShortcut']);self.assertEqual(nk.NativeKeyboard.emit(self,h),[0])
    def test_general_recorded_hotkey_replaces_physical_plan_through_config_staging(self):
        h=self.host();self.general(h);before=h.rpc(dict(op='status'))['result']['keyboardGeneration'];self.sp(h,'settings-record',index=0);self.wait(h,lambda v:v['recording'][0]);self.sp(h,'key-event',code=0,flags=(1<<20)|(1<<19),chars='a');self.wait(h,lambda v:v['surface']['configuration']['launcherHotkey']=='alt+cmd+a');h.settled(lambda s:s['keyboardGeneration']>before and not s['recordingShortcut'])
        self.assertEqual(nk.NativeKeyboard.emit(self,h),[-9874]);self.assertEqual(nk.NativeKeyboard.emit(self,h,code=0,modifiers=2304),[0]);h.settled(lambda s:s['nativeUI']['visible'])
        trace=nk.NativeKeyboard.trace(self,h);self.assertTrue(any(x['phase']=='unregister' for x in trace));self.assertTrue(any(x['phase']=='register' and x['a']==0 and x['b']==2304 for x in trace))
    key=actions.ActionsProcesses.key
    view=actions.ActionsProcesses.view
if __name__=='__main__':unittest.main(verbosity=2)
