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
if __name__=='__main__':unittest.main(verbosity=2)
