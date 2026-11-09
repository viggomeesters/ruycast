"""Main-only choice transition policy; no AppKit app, clipboard, hooks or output.
Native Target/Anchor/monitor/UI integration is separately required.
"""
import copy,json,os,subprocess,unittest
import keyword_process as keyword
ROOT=keyword.ROOT;BIN=ROOT/'target/debug/snippet-choice-probe'
COMMAND=keyword.command('own.é',';own','{field name="name" options="One,Two"}')
def op(name,**values):return dict(op=name,**values)
def visible():return [op('advance'),op('capture'),op('present'),op('activation',sequence=1,actor='own'),op('visible')]
class ChoiceRequest(unittest.TestCase):
    def run_case(self,steps,command=None):
        env={k:v for k,v in os.environ.items() if not k.startswith(('RUYCAST_','RUSTCAST_')) and k!='DYLD_INSERT_LIBRARIES'}
        p=subprocess.run([str(BIN)],input=json.dumps([[op('policy',command=command or COMMAND,steps=steps)]]),text=True,capture_output=True,timeout=10,env=env);self.assertEqual(p.returncode,0,p.stderr);v=json.loads(p.stdout);self.assertTrue(v['ok'],v);return v['cases'][0][0]
    def test_trigger_delay_and_single_use_apply(self):
        v=self.run_case([op('capture'),op('advance',ms=29),op('capture'),op('advance',ms=1),op('capture'),op('present'),op('visible'),op('return'),op('check'),op('apply'),op('advance'),op('check'),op('apply'),op('apply'),op('finish'),op('return')])
        self.assertFalse(v[0]['accepted']);self.assertFalse(v[2]['accepted']);self.assertTrue(v[4]['accepted']);self.assertFalse(v[9]['accepted']);self.assertTrue(v[12]['accepted']);self.assertFalse(v[13]['accepted']);self.assertEqual(v[-1]['phase'],'Finished')
    def test_every_phase_sticky_foreign_away_back(self):
        phases=[[],[op('advance'),op('capture')],[op('advance'),op('capture'),op('present')],visible(),visible()+[op('return')],visible()+[op('return'),op('advance'),op('check'),op('apply')]]
        for prefix in phases:
            for actor in ['foreign','unknown']:
                with self.subTest(prefix=len(prefix),actor=actor):
                    v=self.run_case(prefix+[op('activation',sequence=2,actor=actor),op('activation',sequence=3,actor='original'),op('visible'),op('return'),op('advance'),op('check'),op('apply')]);self.assertTrue(all(s['phase']=='Finished' for s in v[len(prefix):]))
    def test_input_during_pending_capture_present_return_cancels(self):
        for prefix in [[],[op('advance'),op('capture')],[op('advance'),op('capture'),op('present')],visible()+[op('return')]]:
            for kind in ['form','mouse','other']:
                v=self.run_case(prefix+[op('input',sequence=1,kind=kind)]);self.assertEqual(v[-1]['phase'],'Finished')
    def test_visible_own_editing_survives_but_outside_click_does_not(self):
        v=self.run_case(visible()+[op('input',sequence=1,kind='form'),op('input',sequence=2,kind='mouse'),op('outside'),op('visible')]);self.assertEqual(v[-3]['phase'],'Visible');self.assertEqual(v[-1]['phase'],'Finished')
    def test_activation_and_input_sequence_replay_fail_closed(self):
        for suffix in [[op('activation',sequence=1,actor='own')],[op('input',sequence=1,kind='form'),op('input',sequence=1,kind='form')]]:
            self.assertEqual(self.run_case(visible()+suffix)[-1]['phase'],'Finished')
    def test_pending_config_preserves_draft_without_allowing_return(self):
        equivalent=copy.deepcopy(COMMAND);equivalent['id']='own.é'
        v=self.run_case(visible()+[op('pending'),op('input',sequence=1,kind='form'),op('return'),op('config',command=equivalent),op('return'),op('activation',sequence=2,actor='original'),op('advance'),op('check'),op('apply')]);self.assertEqual(v[7]['phase'],'Visible');self.assertFalse(v[7]['accepted']);self.assertTrue(v[8]['accepted']);self.assertTrue(v[-1]['accepted'])
    def test_all_stored_command_fields_and_removed_command_revoke(self):
        candidates=[None]
        for field,value in dict(id='other',title='Changed',kind='url',target='new',aliases=['new'],arguments=['new'],hotkey='cmd+a',timeout=2,inputPrompt='new',keyword='other',applicationID='new',snippetOutput='html',enabled=False).items():
            c=copy.deepcopy(COMMAND);c[field]=value;candidates.append(c)
        for candidate in candidates:
            v=self.run_case(visible()+[op('pending'),op('config',command=candidate),op('pending'),op('config',command=COMMAND)]);self.assertEqual(v[-1]['phase'],'Finished')
    def test_stale_config_ticket_cannot_accept_or_cancel_newer_pending(self):
        stale=copy.deepcopy(COMMAND);stale['title']='stale'
        v=self.run_case(visible()+[op('pending'),op('pending'),op('config',ticket=0,command=stale),op('return'),op('config',ticket=1,command=COMMAND,generation=3),op('config',ticket=1,command=stale),op('return')]);self.assertFalse(v[7]['accepted']);self.assertEqual(v[7]['phase'],'Visible');self.assertFalse(v[8]['accepted']);self.assertTrue(v[9]['accepted']);self.assertFalse(v[10]['accepted']);self.assertTrue(v[11]['accepted'])
    def test_cross_request_capture_and_config_tickets_do_not_authorize_new_request(self):
        v=self.run_case([op('advance'),op('capture'),op('replace'),op('advance'),op('capture',keep=True),op('present')]);self.assertFalse(v[-1]['accepted']);self.assertEqual(v[-1]['phase'],'Capturing')
        v=self.run_case([op('pending'),op('replace'),op('pending'),op('config',ticket=0,command=COMMAND),op('config',ticket=1,command=COMMAND)]);self.assertFalse(v[-2]['accepted']);self.assertTrue(v[-1]['accepted'])
        v=self.run_case(visible()+[op('return'),op('advance'),op('check'),op('replace')]+visible()+[op('return'),op('advance'),op('apply')]);self.assertFalse(v[-1]['accepted']);self.assertEqual(v[-1]['phase'],'Returning')
    def test_sequence_gaps_are_not_silently_coalesced(self):
        for suffix in [[op('activation',sequence=3,actor='own')],[op('input',sequence=2,kind='form')]]:
            v=self.run_case(visible()+suffix);self.assertEqual(v[-1]['phase'],'Finished')
    def test_confirmed_older_generation_cannot_replace_current_certificate(self):
        v=self.run_case(visible()+[op('pending'),op('config',command=COMMAND,generation=3),op('pending'),op('config',command=COMMAND,generation=2)]);self.assertEqual(v[-1]['phase'],'Finished')
    def test_captured_stamp_is_not_refreshed_by_compatible_config(self):
        v=self.run_case([op('advance'),op('capture'),op('pending'),op('config',command=COMMAND),op('present')]);self.assertFalse(v[-1]['accepted']);self.assertEqual(v[-1]['phase'],'Capturing')
    def test_return_original_and_own_allowed_but_later_input_revokes(self):
        v=self.run_case(visible()+[op('return'),op('activation',sequence=2,actor='own'),op('activation',sequence=3,actor='original'),op('advance'),op('check'),op('input',sequence=1,kind='other'),op('apply')]);self.assertEqual(v[-1]['phase'],'Finished');self.assertFalse(v[-1]['accepted'])
    def test_return_has_initial_delay_and_fifteen_retries(self):
        steps=visible()+[op('return'),op('retry')]
        for _ in range(16):steps += [op('advance'),op('retry')]
        v=self.run_case(steps);self.assertFalse(v[6]['accepted']);self.assertEqual(sum(s['accepted'] for s in v[7:]),15);self.assertEqual(v[-1]['phase'],'Finished')
    def test_config_pending_revokes_an_earlier_return_check(self):
        v=self.run_case(visible()+[op('return'),op('advance'),op('check'),op('pending'),op('apply'),op('config',command=COMMAND),op('apply'),op('check'),op('apply')]);self.assertFalse(v[9]['accepted']);self.assertFalse(v[11]['accepted']);self.assertTrue(v[13]['accepted'])
    def test_first_return_poll_starts_after_the_single_activation_completes(self):
        v=self.run_case(visible()+[op('return'),op('advance',ms=100),op('activated-return'),op('activated-return'),op('advance',ms=29),op('retry'),op('advance',ms=1),op('check'),op('apply')]);self.assertTrue(v[7]['accepted']);self.assertFalse(v[7]['returnDue']);self.assertFalse(v[8]['accepted']);self.assertFalse(v[10]['accepted']);self.assertTrue(v[11]['returnDue']);self.assertTrue(v[13]['accepted'])
if __name__=='__main__':unittest.main(verbosity=2)
