"""Own-process actual Carbon ABI, with complete mandatory synthetic SDK.
No physical hotkeys, input posting, frontmost target or personal data.
"""
import ast
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
PROBE=ROOT/'target/debug/keyboard-probe'
SHIM=ROOT/'target/keyboard-shim.dylib'
PARTIAL=ROOT/'target/keyboard-partial.dylib'
DEFAULT=json.loads((ROOT.parent/'config/default.json').read_text())

def config(launcher='cmd+a',clipboard='cmd+s'):
    c=copy.deepcopy(DEFAULT);c.update(launcherHotkey=launcher,clipboardHotkey=clipboard,commands=[]);return c
def stage(c):return dict(action='stage',config=c)
def action(name,**fields):return dict(action=name,**fields)

class Keyboard(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        for path,extra in [(SHIM,[]),(PARTIAL,['-DOMIT_PARAMETER'])]:
            run=subprocess.run(['cc','-Wall','-Wextra','-Werror','-dynamiclib','-framework','Carbon',*extra,str(ROOT/'tests/keyboard_shim.c'),'-o',str(path)],capture_output=True,text=True)
            if run.returncode:raise AssertionError(run.stderr)
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            p=Path(evidence);p.mkdir(parents=True,exist_ok=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            sources=[ROOT/'tests/keyboard_shim.c',ROOT.parent/'Sources/LauncherCore/RustBridge.swift',ROOT.parent/'Sources/RuycastApp/Hotkeys.swift']
            (p/'keyboard-reference-bindings.json').write_text(json.dumps(dict(sources={str(s.relative_to(ROOT.parent)):sha(s) for s in sources},artifacts={str(a.relative_to(ROOT.parent)):sha(a) for a in [SHIM,PARTIAL]}),indent=2))
    def run_probe(self,operations,flags=(),shim=True):
        with tempfile.TemporaryDirectory(prefix='keyboard-',dir=ROOT/'target') as temp:
            home=Path(temp);(home/'.ruycast-keyboard-fixture').write_text('own-keyboard-fixture-v1')
            for flag in flags:(home/flag).write_text('own fault')
            if 'missing-marker' in flags:(home/'.ruycast-keyboard-fixture').unlink()
            env=dict(os.environ)
            for name in ['DYLD_INSERT_LIBRARIES','RUYCAST_HOME','RUYCAST_CONFIG_DIR','RUSTCAST_HOME','RUYCAST_KEYBOARD_FIXTURE_HOME']:env.pop(name,None)
            env.update(HOME=str(home),RUYCAST_KEYBOARD_FIXTURE_HOME=str(home))
            if shim:env['DYLD_INSERT_LIBRARIES']=str(SHIM if shim is True else PARTIAL)
            run=subprocess.run([str(PROBE)],input=json.dumps(operations),env=env,capture_output=True,text=True,timeout=8)
            self.assertEqual(run.returncode,0,run.stdout+run.stderr)
            value=json.loads(run.stdout);path=home/'keyboard-trace.jsonl';trace=[json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
            self.assertTrue(all(e['mainThread'] for e in trace));return value,trace
    def test_complete_boundary_missing_symbols_and_missing_marker_fail_closed(self):
        for shim,flags in [(False,()),('partial',()),(True,['missing-marker'])]:
            value,trace=self.run_probe([],shim=shim,flags=flags);self.assertFalse(value['initialization']['ok']);self.assertEqual(trace,[])
    def test_real_registration_callback_identity_staging_and_retirement(self):
        operations=[stage(config()),action('emit',id=1),action('commit'),action('emit',id=1),stage(config('cmd+d')),action('emit',id=3),action('commit'),action('emit',id=1),action('emit',id=3),action('emit',id=2),action('drop'),action('emit',id=3)]
        v,t=self.run_probe(operations,flags=['emit-on-register','emit-on-unregister'])
        self.assertTrue(v['initialization']['ok']);self.assertEqual([p['action'] for p in v['presses']],['@launcher','@launcher','@clipboard'])
        self.assertEqual([p['generation'] for p in v['presses']],[1,2,2]);self.assertEqual([p['key'] for p in v['presses']],['cmd+a','cmd+d','cmd+s'])
        self.assertTrue(all(e['b']==-9874 for e in t if e['phase'] in ['register-callback','unregister-callback']))
        registrations=[e for e in t if e['phase']=='register'];self.assertEqual([(e['a'],e['b']) for e in registrations],[(0,256),(1,256),(2,256)])
        self.assertEqual(len([e for e in t if e['phase']=='unregister']),3);self.assertEqual([e['phase'] for e in t].count('remove'),1)
    def test_malformed_class_kind_signature_parameter_type_size_and_status_never_dispatch(self):
        variants=[dict(**{key:value}) for key,value in [('class',0),('kind',6),('signature',0),('parameterType',0),('size',4),('size',16),('status',-50)]]
        v,t=self.run_probe([stage(config()),action('commit')]+[action('emit',id=1,**fields) for fields in variants]+[action('emit',id=9999)])
        self.assertEqual(v['presses'],[]);self.assertTrue(all(step.get('status',-9874)==-9874 for step in v['steps'][2:]));self.assertEqual(len([e for e in t if e['phase']=='parameter']),6)
    def test_carbon_modifier_masks_use_current_sdk_constants_and_combined_hyper_mask(self):
        v,t=self.run_probe([stage(config('ctrl+alt+shift+cmd+f12','alt+f1')),action('commit')])
        self.assertTrue(v['steps'][0]['ok']);self.assertEqual({(e['a'],e['b']) for e in t if e['phase']=='register'},{(111,6912),(122,2048)})
    def test_physical_suspension_and_latest_plan_resume_keep_accepted_generation(self):
        v,t=self.run_probe([stage(config()),action('commit'),action('suspend'),action('emit',id=1),stage(config('cmd+d')),action('commit'),action('resume'),action('emit',id=3)])
        self.assertEqual(v['presses'],[dict(generation=2,key='cmd+d',action='@launcher')]);self.assertEqual(len([e for e in t if e['phase']=='register']),4)
    def test_callback_gate_and_panic_are_fail_closed(self):
        v,_=self.run_probe([stage(config()),action('commit'),action('gate',enabled=False),action('emit',id=1),action('gate',enabled=True),action('panic'),action('emit',id=1),action('emit',id=1)])
        self.assertEqual(v['presses'],[]);self.assertEqual([s['status'] for s in v['steps'] if 'status' in s],[-9874]*3)
    def test_partial_registration_and_failed_cleanup_stay_bounded_and_owned(self):
        v,t=self.run_probe([stage(config()),stage(config('cmd+d'))],flags=['partial-register','fail-unregister'])
        self.assertFalse(v['steps'][0]['ok']);self.assertFalse(v['steps'][1]['ok']);self.assertEqual(v['presses'],[])
        self.assertEqual(len([e for e in t if e['phase']=='register']),1);self.assertGreaterEqual(len([e for e in t if e['phase']=='unregister']),2)
    def test_unresolved_old_registration_prevents_further_candidate_registrations(self):
        v,t=self.run_probe([stage(config()),action('commit'),stage(config('cmd+d')),action('commit'),stage(config('cmd+f')),stage(config('cmd+g'))],flags=['fail-unregister'])
        self.assertTrue(v['steps'][2]['ok']);self.assertFalse(v['steps'][4]['ok']);self.assertFalse(v['steps'][5]['ok'])
        self.assertEqual(len([e for e in t if e['phase']=='register']),3)
    def test_failed_remove_and_success_with_null_handler_keep_disabled_callback_userdata(self):
        v,_=self.run_probe([stage(config()),action('commit'),action('drop'),action('gate',enabled=True),action('emit',id=1)],flags=['fail-remove'])
        self.assertEqual(v['presses'],[]);self.assertEqual(v['steps'][-1]['status'],-9874)
        v,t=self.run_probe([action('emit',id=1)],flags=['null-install']);self.assertFalse(v['initialization']['ok']);self.assertEqual(v['presses'],[]);self.assertEqual(v['steps'][0]['status'],-9874)
        self.assertFalse(any(e['phase']=='register' for e in t))
    def test_every_literal_actual_swift_shortcut_keycode_including_escape(self):
        source=(ROOT.parent/'Sources/LauncherCore/RustBridge.swift').read_text()
        literal=re.search(r'keyCodes:\[String:UInt32\]=(\[.*?\])\n',source,re.S).group(1)
        codes=ast.literal_eval('{'+literal[1:-1]+'}')
        for key,code in codes.items():
            if key in ['left-control','left-option']:continue
            with self.subTest(key=key):
                value,trace=self.run_probe([stage(config('ctrl+alt+'+key)),action('commit')])
                self.assertTrue(value['steps'][0]['ok']);self.assertEqual([e['a'] for e in trace if e['phase']=='register' and e['b']==(1<<12|1<<11)],[code])

if __name__=='__main__':unittest.main(verbosity=2)
