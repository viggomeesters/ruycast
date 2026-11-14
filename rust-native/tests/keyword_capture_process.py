"""Complete owned native keyword filter/effects; no real apps/input/clipboard."""
import copy,json,os,subprocess,unittest
import shutil
from pathlib import Path
import runtime_process as rt
ROOT=Path(__file__).resolve().parents[1];BIN=Path(os.environ.get('RUYCAST_KEYWORD_CAPTURE_PROBE',ROOT/'target/debug/keyword-capture-probe'));SDK=ROOT/'target/keyword-capture-fixture'
def config(text='HELLO',keyword='abc',**extra):
    c=copy.deepcopy(rt.DEFAULT);c['commands']=[dict(id='own.snippet',title='Own snippet',kind='snippet',target=text,keyword=keyword,**extra)];c['snippetExpansionEnabled']=True;return c
def sdk(control='state',**kw):return dict(op='sdk',control=control,**kw)
def event(text='',**kw):return sdk('event',text=text,**kw)
def keys(text):return [event(c) for c in text]
class KeywordCapture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        SDK.mkdir(parents=True,exist_ok=True)
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT.parent/'.build/full-rust-release/native'))
        for name,variable,library in [('complete','RUYCAST_TEST_KEYWORD_SHIM','libkeyword_fixture.dylib'),('missing','RUYCAST_TEST_KEYWORD_PARTIAL','libkeyword_missing_value_type.dylib')]:
            artifact=Path(os.environ.get(variable,target/'release/examples'/library))
            if not artifact.is_absolute() or not artifact.is_file():
                raise RuntimeError('Build --features test-tools --example keyword-fixture --example keyword-missing-value-type first')
            shutil.copyfile(artifact,SDK/(name+'.dylib'))
    def invoke(self,ops,lib='complete',ok=True):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        if lib:env['DYLD_INSERT_LIBRARIES']=str(SDK/(lib+'.dylib'))
        p=subprocess.run([str(BIN)],input=json.dumps(ops),capture_output=True,text=True,env=env,timeout=10);self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout);self.assertEqual(v['ok'],ok,v)
        if ok:self.assertTrue(v['retired'],v);self.assertEqual(v['final']['allocated'],0,v);self.assertEqual(v['final']['apps'],0,v)
        return v
    def run_keys(self,text='abc',c=None,between=None):return self.invoke([dict(op='config',value=c or config()),dict(op='refresh'),*(between or []),*keys(text)])
    def inserted(self,state):
        units=[u for e in state['output'] if e['down'] and e['key']==0 for u in e['units']];return bytes(b for u in units for b in [u&255,u>>8]).decode('utf-16-le')
    def test_eventless_recording_and_hyper_episodes_invalidate_prefix(self):
        for middle in [[dict(op='gate',blocked=True,epoch=1),dict(op='gate',blocked=False,epoch=2)],[dict(op='gate',hyperHeld=True,hyperEpoch=1),dict(op='gate',hyperHeld=False,hyperEpoch=2)]]:
            v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),*middle,event('c'),event(''),*keys('abc')]);self.assertFalse(v['steps'][-5]['consumed']);self.assertTrue(v['steps'][-1]['consumed'])
    def test_native_target_instance_switch_resets_even_when_pid_reused(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),sdk('environment',app=2),event('c'),event(''),*keys('abc')]);self.assertFalse(v['steps'][-5]['consumed']);self.assertTrue(v['steps'][-1]['consumed'])
    def test_secure_permission_and_own_app_no_text_read_or_effect(self):
        for env in [dict(flags=8),dict(flags=16),dict(flags=4096),dict(pid='self')]:
            v=self.run_keys(between=[sdk('environment',**env)]);self.assertEqual(v['final']['posts'],0);self.assertEqual(v['final']['valueReads'],0)
    def test_canonical_excluded_app_bundle(self):
        c=config();c['clipboard']['excludedApps']=['own.é']
        v=self.run_keys(c=c,between=[sdk('bundle',value='own.é')]);self.assertEqual(v['final']['posts'],0)
    def test_generation_replacement_resets_and_uses_new_prepared_template(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),dict(op='config',generation=2,value=config('NEW')),dict(op='refresh'),event('c'),event(''),*keys('abc')]);self.assertFalse(v['steps'][-5]['consumed']);self.assertTrue(v['steps'][-1]['consumed']);self.assertEqual(self.inserted(v['final']),'NEW')
    def test_catalog_adoption_keeps_observation_tap_but_revokes_old_prefix(self):
        changed=config('NEW');changed['theme']='dark'
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),dict(op='config',generation=2,value=changed),sdk(),event('c'),dict(op='refresh'),event(''),*keys('abc')])
        self.assertTrue(v['steps'][5]['tapValid']);self.assertTrue(v['steps'][5]['tapEnabled'])
        self.assertFalse(v['steps'][6]['consumed']);self.assertTrue(v['steps'][-1]['consumed']);self.assertEqual(self.inserted(v['final']),'NEW');self.assertEqual(v['final']['tapCreates'],1)
    def test_pending_generation_keeps_existing_tap_but_no_effects(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),dict(op='gate',generation=0),dict(op='refresh'),sdk(),*keys('abc'),dict(op='gate',generation=1),dict(op='refresh'),event('c'),event(''),*keys('abc')]);self.assertTrue(v['steps'][6]['tapValid']);self.assertTrue(v['steps'][6]['tapEnabled']);self.assertFalse(any(x.get('consumed') for x in v['steps'][7:10]));self.assertFalse(v['steps'][12]['consumed']);self.assertTrue(v['steps'][-1]['consumed']);self.assertEqual(v['final']['tapCreates'],1)
    def test_sampled_tap_loss_cannot_restore_old_prefix_after_recreate(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),sdk('failure',name='tap_enabled'),dict(op='refresh'),sdk('failure'),dict(op='refresh'),event('c'),event(''),*keys('abc')]);self.assertFalse(v['steps'][8]['consumed']);self.assertTrue(v['steps'][-1]['consumed'])
    def test_stage_episode_not_seen_by_refresh_still_revokes_prefix(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),dict(op='gate',stageEpoch=1),event('c'),event(''),*keys('abc')]);self.assertFalse(v['steps'][5]['consumed']);self.assertTrue(v['steps'][-1]['consumed']);self.assertEqual(v['final']['tapCreates'],1)
    def test_off_and_paused_short_circuit_permission_app_discovery(self):
        for ops in [[dict(op='config',value={**config(),'snippetExpansionEnabled':False})],[dict(op='config',value=config()),dict(op='gate',running=False)]]:
            v=self.invoke([*ops,sdk('environment',flags=8192|4096),dict(op='refresh')]);self.assertEqual(v['final']['calls'],0);self.assertEqual(v['final']['tapCreates'],0)
    def test_interactive_trigger_passes_without_synchronous_effects(self):
        v=self.run_keys(c=config('{name options="One, Two"}'));self.assertFalse(v['steps'][-1]['consumed']);self.assertEqual(v['final']['posts'],0)
    def test_allocation_failure_before_first_down_does_not_consume(self):
        for name in ['event:down','event:up']:
            v=self.run_keys(between=[sdk('failure',name=name)]);self.assertFalse(v['steps'][-1]['consumed']);self.assertEqual(v['final']['posts'],0)
    def test_secure_native_change_after_entered_down_balances_without_further_text(self):
        for flags in [8,16,4096]:
            v=self.run_keys(between=[sdk('change',at='post:down',flags=flags)]);self.assertTrue(v['steps'][-1]['consumed']);self.assertEqual(len(v['final']['output']),2);self.assertEqual(v['final']['output'][-1]['units'],[]);self.assertFalse(v['final']['output'][-1]['down'])
    def test_last_policy_call_cannot_redirect_first_post_to_another_app(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),sdk('change',at='copy:1',nth=4,app=2),event('c')]);self.assertFalse(v['steps'][-1]['consumed']);self.assertEqual(v['final']['posts'],0)
    def test_recording_or_reentry_after_entered_down_cannot_lose_balance(self):
        for action in ['record','reenter','drop','panic','stop','disable']:
            v=self.run_keys(between=[dict(op='hook',at='post:down',action=action)]);self.assertTrue(v['steps'][-1]['consumed'],action);self.assertEqual(len(v['final']['output']),2,action);self.assertFalse(v['final']['output'][-1]['down']);self.assertEqual(v['final']['output'][-1]['units'],[])
    def test_reentrant_disabled_config_never_reauthorizes_setup(self):
        for phase in ['raycast','trusted','runloop','common','tap_create','source','add_source','tap_enable','tap_enabled']:
            v=self.invoke([dict(op='config',value=config()),dict(op='hook',at=phase,action='disable'),dict(op='refresh'),sdk()]);self.assertNotEqual(v['steps'][-2]['status'],'On',phase);self.assertFalse(v['steps'][-1]['tapValid'],phase)
    def test_callback_timeout_revokes_and_refresh_recovers(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),*keys('ab'),event(type=0xffffffff),event('c'),dict(op='refresh'),*keys('abc')]);self.assertFalse(v['steps'][-5]['consumed']);self.assertTrue(v['steps'][-1]['consumed'])
    def test_failed_invalidation_retains_callback_until_recovery(self):
        v=self.invoke([dict(op='config',value=config()),dict(op='refresh'),sdk('failure',name='invalidate'),dict(op='stop'),dict(op='drop'),sdk(),sdk('failure'),dict(op='recover'),sdk()]);self.assertFalse(v['steps'][3]['retired']);self.assertTrue(v['steps'][5]['tapValid']);self.assertTrue(v['steps'][-2]['retired']);self.assertFalse(v['steps'][-1]['tapValid'])
if __name__=='__main__':unittest.main(verbosity=2)
