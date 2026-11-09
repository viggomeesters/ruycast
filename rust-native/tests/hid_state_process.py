"""Actual current Swift pure bodies vs Rust. No HID, listener or event posting.

Only pure bodies are extracted verbatim; constructing HyperKey would install
observers/timers. This is a state/function differential, not the native service.
"""
import hashlib
import itertools
import json
import os
from pathlib import Path
import random
import re
import subprocess
import unittest

ROOT=Path(__file__).resolve().parents[1]
REPO=ROOT.parent
DIR=ROOT/'target/hid-state-reference'
REFERENCE=DIR/'reference'
PROBE=ROOT/'target/debug/hid-state-probe'
CMD=1<<20;CTRL=1<<18;ALT=1<<19;SHIFT=1<<17;FN=1<<23;CAPS=1<<16

def physical(usage,down,time=0,device=1):return dict(op='input',usage=usage,down=down,time=time,device=device)
def event(key=40,flags=CMD|16,kind=10,repeat=False):return dict(op='event',key=key,flags=flags,kind=kind,repeat=repeat)
def bindings(*items):return dict(op='bindings',bindings=[dict(shortcut=s,action=a) for s,a in items])
def block(text,start,end):
    a=text.index(start);b=text.index(end,a)
    return text[a:b]

class HIDState(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.comparisons=[];DIR.mkdir(exist_ok=True)
        source=(REPO/'Sources/RuycastApp/HyperKey.swift').read_text()
        snippet=(REPO/'Sources/RuycastApp/SnippetExpander.swift').read_text()
        filter_body=source[source.index('struct CommandShortcutFilter {'):]
        flags_body=block(source,'    static func generated(','    private func emitModifiers(')
        modifier_body=block(source,'    private var physicalModifiers:CGEventFlags {','    private func clearCapsLock(')
        tag=re.search(r'nonisolated static let generatedTag:Int64=(0x[0-9A-Fa-f]+)',source).group(1)
        snippet_tag=re.search(r'nonisolated static let generatedTag:Int64=(0x[0-9A-Fa-f]+)',snippet).group(1)
        hyper_line=next(s.strip() for s in source.splitlines() if s.strip().startswith('static let hyper:'))
        generated='import Foundation\nimport CoreGraphics\nimport IOKit.hidsystem\nimport LauncherCore\n'
        generated+='enum SnippetExpander {static let generatedTag:Int64='+snippet_tag+'}\n'
        generated+='enum FlagReference {static let generatedTag:Int64='+tag+'\n'+hyper_line+'\n'+flags_body+'}\n'
        readiness_body=block(source,'        let reason:String\n','        if !reason.isEmpty')
        generated+='@MainActor enum ReadinessPredicates {static var facts:[String:Any]=[:]}\n'
        generated+='@MainActor enum NSRunningApplication {static func runningApplications(withBundleIdentifier:String)->[Int] {ReadinessPredicates.facts["raycast"] as! Bool ? [1]:[]}}\n'
        generated+='@MainActor func IsSecureEventInputEnabled()->Bool {ReadinessPredicates.facts["secure"] as! Bool}\n'
        generated+='@MainActor func AXIsProcessTrusted()->Bool {ReadinessPredicates.facts["accessibility"] as! Bool}\n'
        generated+='@MainActor func CGPreflightListenEventAccess()->Bool {ReadinessPredicates.facts["monitoring"] as! Bool}\n'
        generated+='@MainActor struct ReadinessReference {let hyperEnabled:Bool;let bindings:[UInt32:String];let commandBindings:[String:String];let sleeping:Bool;func reason()->String {\n'+readiness_body+'return reason\n}}\n'
        generated+='struct PhysicalModifierReference {let state:KeyboardState; static func flags(_ s:KeyboardState)->CGEventFlags {Self(state:s).physicalModifiers}\n'+modifier_body+'}\n'+filter_body
        for forbidden in ['tapCreate','IOHIDManagerCreate','CGEventPost','NSWorkspace','Timer','IOServiceOpen']:
            if forbidden in generated:raise AssertionError('Pure oracle included native effect: '+forbidden)
        (DIR/'actual-pure-bodies.swift').write_text(generated)
        cls.extractions={name:hashlib.sha256(body.encode()).hexdigest() for name,body in [('filter',filter_body),('flags',flags_body),('modifiers',modifier_body),('readiness',readiness_body)]}
        commands=[
            ['swiftc','-emit-library','-emit-module','-module-name','LauncherModels','-o',str(DIR/'libLauncherModels.dylib'),'-emit-module-path',str(DIR/'LauncherModels.swiftmodule'),str(REPO/'Sources/LauncherModels/Models.swift')],
            ['swiftc','-emit-library','-emit-module','-module-name','LauncherCore','-I',str(DIR),'-L',str(DIR),'-lLauncherModels','-I',str(REPO/'.build/swift/debug/CRuycastCore.build'),'-L',str(REPO/'.build/rust/release'),'-lrustcast_core','-liconv','-lresolv','-Xlinker','-rpath','-Xlinker',str(DIR),'-o',str(DIR/'libLauncherCore.dylib'),'-emit-module-path',str(DIR/'LauncherCore.swiftmodule'),*[str(REPO/p) for p in ['Sources/LauncherCore/Files.swift','Sources/LauncherCore/RustBridge.swift','Sources/LauncherCore/KeyboardState.swift']]],
            ['swiftc','-parse-as-library','-I',str(REPO/'.build/swift/debug/CRuycastCore.build'),'-I',str(DIR),'-L',str(DIR),'-lLauncherModels','-lLauncherCore','-Xlinker','-rpath','-Xlinker',str(DIR),str(DIR/'actual-pure-bodies.swift'),str(ROOT/'tests/hid_state_reference.swift'),'-o',str(REFERENCE)],
        ]
        for cmd in commands:
            run=subprocess.run(cmd,capture_output=True,text=True,timeout=90)
            if run.returncode:raise AssertionError(run.stderr)
        cls.source_paths=[REPO/p for p in ['Sources/LauncherModels/Models.swift','Sources/LauncherCore/KeyboardState.swift','Sources/LauncherCore/Files.swift','Sources/LauncherCore/RustBridge.swift','Sources/RuycastApp/HyperKey.swift','Sources/RuycastApp/SnippetExpander.swift','Sources/RuycastApp/KeyboardChecks.swift','Sources/RuycastApp/SettingsChecks.swift']]+[ROOT/'tests/hid_state_reference.swift',ROOT/'tests/hid_state_process.py']
        cls.artifact_paths=[REFERENCE,DIR/'libLauncherCore.dylib',DIR/'libLauncherModels.dylib',DIR/'actual-pure-bodies.swift',REPO/'.build/rust/release/librustcast_core.a',REPO/'.build/swift/debug/CRuycastCore.build/module.modulemap']
    @classmethod
    def tearDownClass(cls):
        if evidence:=os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR'):
            out=Path(evidence);out.mkdir(parents=True,exist_ok=True);sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
            (out/'hid-state-reference-bindings.json').write_text(json.dumps(dict(scope='Actual literal Swift pure bodies only; no whole HID/Hyper service acceptance',sources={str(p.relative_to(REPO)):sha(p) for p in cls.source_paths},artifacts={str(p.relative_to(REPO)):sha(p) for p in cls.artifact_paths},extractions=cls.extractions),indent=2))
            (out/'hid-state-comparisons.json').write_text(json.dumps(dict(comparedSteps=sum(sum(len(case) for case in c['cases']) for c in cls.comparisons),comparisons=cls.comparisons),ensure_ascii=False))
    def invoke(self,binary,cases):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        run=subprocess.run([str(binary)],input=json.dumps(cases,allow_nan=False),env=env,capture_output=True,text=True,timeout=15)
        self.assertEqual(run.returncode,0,run.stdout[:1500]+run.stderr)
        v=json.loads(run.stdout);self.assertTrue(v['ok']);return v['cases']
    def compare(self,cases):
        expected=self.invoke(REFERENCE,cases);actual=self.invoke(PROBE,cases)
        self.assertEqual(len(actual),len(cases));self.assertEqual(len(expected),len(cases))
        for i,(before,after) in enumerate(zip(expected,actual)):
            self.assertEqual(len(before),len(cases[i]));self.assertEqual(len(after),len(before))
            for j,(a,b) in enumerate(zip(before,after)):self.assertEqual(a,b,json.dumps(dict(case=i,step=j,op=cases[i][j],swift=a,rust=b)))
        self.comparisons.append(dict(test=self.id(),cases=cases,expected=expected,actual=actual));return actual
    def test_physical_edges_duplicates_timing_nonfinite_and_stray_events(self):
        cases=[]
        for usage in [0xe0,0xe2,0xe1,0xe7,0x39,0,2**32-1]:
            for end in [.5,.500000000001,.499999999999,-.001,'nan','inf','-inf']:
                cases.append([physical(usage,True),physical(usage,True,.1),physical(9,False,.2,99),physical(usage,False,end),dict(op='reset')])
        for start in ['nan','inf','-inf']:
            cases.append([physical(0xe0,True,start),physical(0xe0,False,.1)])
        self.compare(cases)
    def test_multiple_devices_chords_cancel_remove_and_modifier_masks(self):
        cases=[]
        for device in [0,1,2**64-1]:
            for usage in range(0xe0,0xe8):
                cases.append([physical(0x39,True,device=device),physical(0x39,True,device=7),physical(usage,True,.1,device),physical(0x39,False,.2,device),dict(op='remove',device=7),dict(op='remove',device=device)])
        for op in [dict(op='cancel'),dict(op='remove',device=99),physical(4,True,.1,2),physical(0xe7,True,.1,2),dict(op='reset')]:
            cases.append([physical(0xe0,True),op,physical(0xe0,False,.2)])
        result=self.compare(cases);self.assertTrue(result[0][3]['caps']);self.assertEqual(result[0][-1]['held'],[])
    def test_generated_flags_rcmd_unknown_bits_and_all_modifier_combinations(self):
        flags=[sum(b for b,on in zip([CMD,CTRL,ALT,SHIFT,FN,CAPS,8,16,1<<40],bits) if on) for bits in itertools.product([False,True],repeat=9)]
        cases=[[dict(op='rewrite',flags=f,held=held,tag=tag)] for f,held,tag in itertools.product(flags,[False,True],[0,0x52554859504552,0x5255535443415354,-1])]
        self.compare(cases)
        self.compare([[dict(op='rewrite',flags=2**64-1,held=held,tag=tag)] for held,tag in itertools.product([False,True],[0,-2**63])])
    def test_oracle_contains_no_native_hook_sdk_imports(self):
        forbidden={'_CGEventTapCreate','_CGEventPost','_IOServiceOpen','_IOHIDManagerCreate','_IOHIDSetModifierLockState','_CGPreflightListenEventAccess','_AXIsProcessTrusted','_IsSecureEventInputEnabled','_OBJC_CLASS_$_NSWorkspace','_OBJC_CLASS_$_NSRunningApplication'}
        for binary in [REFERENCE,DIR/'libLauncherCore.dylib']:
            run=subprocess.run(['nm','-u',str(binary)],capture_output=True,text=True,check=True)
            self.assertFalse(forbidden.intersection(line.split()[-1] for line in run.stdout.splitlines() if line.split()),str(binary))
    def test_sided_binding_keycodes_exact_modifiers_and_swallowed_release(self):
        source=(REPO/'Sources/LauncherCore/RustBridge.swift').read_text();literal=source.split('public static let keyCodes:',1)[1].split('public init(',1)[0]
        keys=re.findall(r'"((?:\\.|[^"\\])*)":\d+',literal)
        names=[]
        for s in keys:
            name=s.replace("\\'", "'").replace('\\\\','\\')
            if name not in names:names.append(name)
        cases=[]
        for key in names:
            for side,mask in [('right-cmd',16),('left-cmd',8)]:
                code=next(int(m.group(1)) for m in re.finditer(r'"'+re.escape(key.replace('\\','\\\\').replace("'","\\'"))+r'":(\d+)',literal))
                bind=bindings((side+'+'+key,'exact'))
                cases.append([bind,event(code,CMD),event(code,CMD|24),event(code,CMD|mask|FN),event(code,CMD|mask|ALT),event(code,CMD|mask,repeat=True),event(code,CMD|mask),event(code,0,repeat=True),bindings(),event(code,0,11),event(code,0,11)])
        self.compare(cases)
    def test_permission_and_recovery_reason_precedence_is_not_activation(self):
        keys=['hyper','modifiers','sided','sleeping','raycast','secure','accessibility','monitoring']
        cases=[[dict(op='readiness',**dict(zip(keys,values)))] for values in itertools.product([False,True],repeat=8)]
        self.compare(cases)
    def test_sequence_oracle_fuzz_and_first_duplicate_binding_wins(self):
        rng=random.Random(0x525548)
        cases=[]
        for _ in range(48):
            case=[bindings(('right-cmd+k','first'),('right-cmd+k','second'),('shift+left-cmd+k','left'))]
            for index in range(96):
                op=rng.randrange(9)
                if op<3:case.append(physical(rng.choice([0x39,4,7,0xe0,0xe2,0xe7]),bool(rng.getrandbits(1)),index/10,rng.randrange(3)))
                elif op<6:case.append(event(rng.choice([40,41]),rng.choice([CMD|16,CMD|8|SHIFT,CMD,CMD|24,0]),rng.choice([10,11,12,1,22]),bool(rng.getrandbits(1))))
                elif op==6:case.append(dict(op='remove',device=rng.randrange(3)))
                else:case.append(dict(op=rng.choice(['cancel','filter-reset','reset'])))
            cases.append(case)
        self.compare(cases)

if __name__=='__main__':unittest.main(verbosity=2)
