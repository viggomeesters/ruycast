"""Complete own SDK/app/AX boundary, no real app discovery or input effects."""
import json,os,subprocess,unittest
import shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];DIR=ROOT/'target/keyword-native-fixture';BIN=Path(os.environ.get('RUYCAST_KEYWORD_NATIVE_PROBE',ROOT/'target/debug/keyword-native-probe'))
class KeywordNative(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIR.mkdir(parents=True,exist_ok=True)
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT.parent/'.build/full-rust-release/native'))
        for name,variable,library in [('complete','RUYCAST_TEST_KEYWORD_SHIM','libkeyword_fixture.dylib'),('missing','RUYCAST_TEST_KEYWORD_PARTIAL','libkeyword_missing_value_type.dylib')]:
            artifact=Path(os.environ.get(variable,target/'release/examples'/library))
            if not artifact.is_absolute() or not artifact.is_file():
                raise RuntimeError('Build --features test-tools --example keyword-fixture --example keyword-missing-value-type first')
            shutil.copyfile(artifact,DIR/(name+'.dylib'))
    def invoke(self,ops,lib='complete',ok=True):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        if lib:env['DYLD_INSERT_LIBRARIES']=str(DIR/(lib+'.dylib'))
        p=subprocess.run([str(BIN)],input=json.dumps(ops),capture_output=True,text=True,env=env,timeout=10)
        self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout);self.assertEqual(v['ok'],ok,v)
        if ok:self.assertEqual(v['final']['allocated'],0,v);self.assertEqual(v['final']['apps'],0,v)
        return v
    def test_complete_table_required_before_any_real_discovery(self):
        for lib in [None,'missing']:
            self.assertIn('Complete keyword SDK fixture',self.invoke([dict(op='front')],lib,False)['error'])
    def test_mixed_backend_refuses_anchor_and_effects_before_sdk_calls(self):
        v=self.invoke([dict(op='front'),dict(op='state'),dict(op='mixed-backend'),dict(op='state')])['steps'];self.assertEqual(v[1],v[3]);self.assertEqual(v[2],dict(plain=False,rich=False,cursor=False,keyboardPossible=False,clipboardPossible=False,anchorRejected=True))
    def test_retained_instance_not_pid_or_bundle_replacement(self):
        v=self.invoke([dict(op='front'),dict(op='current'),dict(op='env',app=2),dict(op='current'),dict(op='env',pid=43),dict(op='current'),dict(op='env',flags=4),dict(op='current'),dict(op='activate')])['steps']
        self.assertTrue(v[1]['current']);self.assertFalse(v[3]['current']);self.assertFalse(v[5]['current']);self.assertFalse(v[7]['current']);self.assertFalse(v[8]['activated'])
    def test_missing_target_and_bundle(self):
        v=self.invoke([dict(op='env',flags=1),dict(op='front'),dict(op='env',flags=2),dict(op='front')])['steps'];self.assertIsNone(v[1]['target']);self.assertIsNone(v[3]['target']['bundle'])
    def test_plain_secure_checks_never_read_editor_value(self):
        for flags,safe in [(0,True),(8,False),(16,False),(4096,False),(32,True),(64,True),(128,True)]:
            v=self.invoke([dict(op='env',flags=flags),dict(op='safe')]);self.assertEqual(v['steps'][-1],dict(safe=safe));self.assertEqual(v['final']['valueReads'],0)
    def test_explicit_timeout_failure_stops_before_copy(self):
        for name,count in [('timeout:system',0),('timeout:element',1)]:
            v=self.invoke([dict(op='failure',name=name),dict(op='safe')]);self.assertEqual(v['steps'][-1],dict(error='TimeoutBudget'));self.assertEqual(v['final']['copies'],count)
    def test_anchor_identity_value_and_range_cannot_degrade(self):
        for change in [dict(op='env',element=2),dict(op='env',flags=32),dict(op='env',flags=64),dict(op='env',flags=256),dict(op='env',flags=512),dict(op='env',flags=1024),dict(op='text',value='abd'),dict(op='text',value='abc',at=2)]:
            v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='anchor'),dict(op='verify'),change,dict(op='verify')])['steps'];self.assertEqual(v[2],dict(ok=True));self.assertEqual(v[3],dict(continuity='pinned'));self.assertEqual(v[-1],dict(continuity='changed'),change)
    def test_initial_missing_element_requires_event_guards(self):
        v=self.invoke([dict(op='env',flags=32),dict(op='front'),dict(op='anchor'),dict(op='verify')])['steps'];self.assertEqual(v[-1],dict(continuity='event-guards-required'))
    def test_initial_partial_metadata_is_preserved_without_new_requirement(self):
        for flags in [256,512,1024,256|1024]:
            v=self.invoke([dict(op='text',value='abc'),dict(op='env',flags=flags),dict(op='front'),dict(op='anchor'),dict(op='env'),dict(op='verify')])['steps'];self.assertEqual(v[-1],dict(continuity='pinned'),flags)
    def test_range_validation_and_exact_utf16_scalar_boundary(self):
        for at,length in [(-1,0),(4,0),(2,1)]:
            v=self.invoke([dict(op='text',value='abc',at=at,length=length),dict(op='front'),dict(op='anchor')])['steps'];self.assertEqual(v[-1],dict(error='TargetChanged'))
        v=self.invoke([dict(op='text',units=[0xd83d,0xde00],at=1),dict(op='front'),dict(op='anchor',keyword='\ufffd')])['steps'];self.assertEqual(v[-1],dict(error='TargetChanged'))
    def test_canonical_value_equality_but_original_utf16_range(self):
        v=self.invoke([dict(op='text',value='abce\u0301',at=3),dict(op='front'),dict(op='anchor'),dict(op='text',value='abcé',at=3),dict(op='verify')])['steps'];self.assertEqual(v[2],dict(ok=True));self.assertEqual(v[-1],dict(continuity='pinned'))
    def test_canonical_editor_value_cannot_move_keyword_behind_same_utf16_caret(self):
        v=self.invoke([dict(op='text',value='e\u0301abcZZ',at=5),dict(op='front'),dict(op='anchor'),dict(op='text',value='éabcZZ',at=5),dict(op='verify')])['steps']
        self.assertEqual(v[2],dict(ok=True));self.assertEqual(v[-1],dict(continuity='changed'))
    def test_ax_factory_failures_do_not_become_metadata_fallback(self):
        v=self.invoke([dict(op='failure',name='system'),dict(op='safe')])['steps'];self.assertEqual(v[-1],dict(error='InvalidData'))
        v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='failure',name='ax_app'),dict(op='anchor')])['steps'];self.assertEqual(v[-1],dict(error='InvalidData'))
    def test_error_outvalues_retire_and_known_value_never_becomes_unknown(self):
        v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='anchor'),dict(op='env',flags=2048),dict(op='failure',name='copy:3'),dict(op='verify')]);self.assertEqual(v['steps'][-1],dict(continuity='changed'))
    def test_reentrant_revocation_releases_every_owned_intermediate(self):
        for phase in ['front','app_pid','app_bundle','type_id','string_type','string_len','string_units','release']:
            v=self.invoke([dict(op='revoke',at=phase),dict(op='front')]);self.assertEqual(v['steps'][-1],dict(error='Revoked'),phase)
        for phase in ['app_equal','app_release','app_terminated']:
            v=self.invoke([dict(op='front'),dict(op='revoke',at=phase),dict(op='current')]);self.assertEqual(v['steps'][-1],dict(error='Revoked'),phase)
        for phase in ['system','timeout:system','copy:0','ax_type','timeout:element','copy:1','string_units','release','trusted','secure']:
            v=self.invoke([dict(op='revoke',at=phase),dict(op='safe')]);self.assertEqual(v['steps'][-1],dict(error='Revoked'),phase)
    def test_native_permission_change_during_ax_copy_is_rechecked(self):
        for flags in [8,4096]:
            for phase in ['copy:0','copy:1','string_units','release']:
                v=self.invoke([dict(op='change',at=phase,flags=flags),dict(op='safe')])
                self.assertEqual(v['steps'][-1],dict(safe=False),(flags,phase))
        for flags in [8,4096]:
            v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='anchor'),dict(op='change',at='copy:3',flags=flags),dict(op='verify')])
            self.assertEqual(v['steps'][-1],dict(error='UnsafeInput'))
    def test_sensitive_value_is_not_read_after_observed_native_permission_change(self):
        for flags in [8,16,4096]:
            for phase in ['copy:2','value_range']:
                v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='change',at=phase,flags=flags),dict(op='anchor')])
                self.assertEqual(v['steps'][-1],dict(error='UnsafeInput'))
                self.assertEqual(v['final']['valueReads'],0,(flags,phase,v))
                v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='anchor'),dict(op='change',at=phase,flags=flags),dict(op='verify')])
                self.assertEqual(v['steps'][-1],dict(error='UnsafeInput'))
                self.assertEqual(v['final']['valueReads'],1,(flags,phase,v))
    def test_native_app_switch_during_anchor_value_read_is_rejected(self):
        v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='change',at='copy:3',app=2),dict(op='anchor')])
        self.assertEqual(v['steps'][-1],dict(error='TargetChanged'))
        v=self.invoke([dict(op='text',value='abc'),dict(op='front'),dict(op='anchor'),dict(op='change',at='copy:3',app=2),dict(op='verify')])
        self.assertEqual(v['steps'][-1],dict(continuity='changed'))
    def test_negative_native_string_length_fails_closed(self):
        v=self.invoke([dict(op='failure',name='string_len'),dict(op='front')])
        self.assertEqual(v['steps'][-1],dict(error='InvalidData'))
    def test_anchor_contents_never_enter_status_or_errors(self):
        secret='OWN-AX-VALUE-NEVER-LEAVES-PROBE abc'
        v=self.invoke([dict(op='text',value=secret),dict(op='front'),dict(op='anchor'),dict(op='verify')]);self.assertNotIn(secret,json.dumps(v));self.assertEqual(v['steps'][-1],dict(continuity='pinned'))
    def test_malformed_known_ax_values_cannot_collapse_to_same_certificate(self):
        for units,at in [([0xd800,97,98,99],4),([97,98,99,0xd800],3),([0xdc00,97,98,99],4),([97,98,99,0xdc00],3)]:
            v=self.invoke([dict(op='text',units=units,at=at),dict(op='front'),dict(op='anchor')]);self.assertEqual(v['steps'][-1],dict(error='InvalidData'))
        for text,units,at in [('Xabc',[0xd801,97,98,99],4),('abcX',[97,98,99,0xd801],3)]:
            v=self.invoke([dict(op='text',value=text,at=at),dict(op='front'),dict(op='anchor'),dict(op='text',units=units,at=at),dict(op='verify')]);self.assertEqual(v['steps'][-1],dict(error='InvalidData'))
    def test_valid_pair_across_stack_chunks_and_malformed_edges(self):
        text='a'*4095+'😀abc';v=self.invoke([dict(op='text',value=text),dict(op='front'),dict(op='anchor'),dict(op='verify')]);self.assertEqual(v['steps'][-1],dict(continuity='pinned'))
        for units in [[97]*4095+[0xd800]+[98,97,98,99],[97]*4096+[0xdc00,97,98,99],[97,98,99]+[97]*4092+[0xd800]]:
            v=self.invoke([dict(op='text',units=units),dict(op='front'),dict(op='anchor')]);self.assertEqual(v['steps'][-1],dict(error='InvalidData'))
    def test_large_valid_ax_snapshot_and_revoke_during_value_chunk(self):
        value='a'*1000000+'abc';v=self.invoke([dict(op='text',value=value),dict(op='front'),dict(op='anchor'),dict(op='verify')]);self.assertEqual(v['steps'][-1],dict(continuity='pinned'));self.assertGreater(v['final']['valueUnitReads'],400)
        v=self.invoke([dict(op='text',value=value),dict(op='front'),dict(op='revoke',at='value_units'),dict(op='anchor')]);self.assertEqual(v['steps'][-1],dict(error='Revoked'));self.assertEqual(v['final']['valueUnitReads'],1)
    def test_general_text_keeps_lossy_semantics_across_stack_chunks(self):
        for units in [[0xd800],[0xdc00],[0xd800,97,0xdc00],[0xd800,0xd83d,0xde00],[97]*4095+[0xd83d,0xde00,0xdc00]]:
            expected=b''.join(v.to_bytes(2,'little') for v in units).decode('utf-16-le',errors='replace');v=self.invoke([dict(op='bundle-units',units=units),dict(op='front')]);self.assertEqual(v['steps'][-1]['target']['bundle'],expected)
    def test_unaddressable_known_value_length_refuses_before_value_units(self):
        v=self.invoke([dict(op='front'),dict(op='text',value='abc'),dict(op='failure',name='value_len_huge'),dict(op='anchor')]);self.assertEqual(v['steps'][-1],dict(error='InvalidData'));self.assertEqual(v['final']['valueUnitReads'],0)
if __name__=='__main__':unittest.main(verbosity=2)
