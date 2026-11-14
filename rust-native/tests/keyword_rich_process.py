"""Rich keyword clipboard+native issuance and original-lease delayed cursor."""
import unittest
import keyword_capture_process as capture
from keyword_capture_process import config,sdk,event,keys
class KeywordRich(unittest.TestCase):
    @classmethod
    def setUpClass(cls):capture.KeywordCapture.setUpClass.__func__(cls)
    invoke=capture.KeywordCapture.invoke
    def rich(self,text='HELLO',mode='html',keyword='abc',before=None,after=None):
        return self.invoke([dict(op='config',value=config(text,keyword=keyword,snippetOutput=mode)),dict(op='refresh'),*(before or []),*keys(keyword),*(after or []),dict(op='status')])
    def text(self,value):return bytes(value or []).decode()
    def test_rich_cursor_is_delayed_and_not_sent_with_paste(self):
        v=self.rich('A{cursor}BC',after=[dict(op='status'),dict(op='wait',ms=100),dict(op='pump')]);self.assertTrue(v['steps'][-4]['cursor']);self.assertFalse(v['steps'][-2]['pendingCursor']);self.assertEqual(sum(e['key']==123 and e['down'] for e in v['final']['output']),2)
    def test_cursor_cancelled_by_input_record_hyper_activation_generation_and_target(self):
        changes=[event('z'),dict(op='gate',epoch=1),dict(op='gate',hyperEpoch=1),dict(op='gate',activationEpoch=1),dict(op='gate',generation=2),dict(op='gate',stageEpoch=1),sdk('environment',app=2),sdk('environment',flags=8),sdk('environment',flags=16),sdk('environment',flags=4096),dict(op='stop')]
        for change in changes:
            v=self.rich('A{cursor}BC',after=[change,dict(op='wait',ms=100),dict(op='pump')]);self.assertEqual(sum(e['key']==123 and e['down'] for e in v['final']['output']),0,change)
    def test_sampled_loss_or_pending_rollback_never_revives_old_cursor(self):
        for changes in [[sdk('failure',name='tap_enabled'),dict(op='refresh'),sdk('failure'),dict(op='refresh')],[dict(op='gate',generation=0),dict(op='refresh'),dict(op='gate',generation=1),dict(op='refresh')],[dict(op='gate',stageEpoch=1)]]:
            v=self.rich('A{cursor}BC',after=[*changes,dict(op='wait',ms=100),dict(op='pump')]);self.assertEqual(sum(e['key']==123 and e['down'] for e in v['final']['output']),0)
    def test_own_generated_input_and_unrelated_clipboard_copy_do_not_cancel_issued_paste_cursor(self):
        for change in [event('z',tag=0x5255535443415354),sdk('board-external',value='LATER COPY')]:
            v=self.rich('A{cursor}BC',after=[change,dict(op='wait',ms=100),dict(op='pump')]);self.assertEqual(sum(e['key']==123 and e['down'] for e in v['final']['output']),2,change)
    def test_large_cursor_runs_bounded_batches_and_allows_cancellation_between_them(self):
        v=self.rich('{cursor}'+'x'*20000,keyword='x',after=[dict(op='wait',ms=100),dict(op='pump'),event('z'),dict(op='pump')]);arrows=sum(e['key']==123 and e['down'] for e in v['final']['output']);self.assertGreater(arrows,0);self.assertLessEqual(arrows,32);self.assertFalse(v['steps'][-2]['pendingCursor'])
    def test_cursor_panic_balances_keyup_and_poison_only_keyword_component(self):
        v=self.rich('A{cursor}BC',after=[dict(op='hook',at='post:down',action='panic'),dict(op='wait',ms=100),dict(op='pump')]);out=[e for e in v['final']['output'] if e['key']==123];self.assertEqual(len(out),2);self.assertFalse(out[-1]['down']);self.assertEqual(v['steps'][-1]['status'],'Keyword callback failed')
if __name__=='__main__':unittest.main(verbosity=2)
