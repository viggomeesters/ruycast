"""Private Foundation center, retained own-process app, no focus/input."""
import unittest
import keyword_native_process as target

class WorkspaceActivation(unittest.TestCase):
    setUpClass = classmethod(target.KeywordNative.setUpClass.__func__)
    invoke = target.KeywordNative.invoke
    def run_case(self, **case):
        return self.invoke([dict(op='activation', case=case)])['steps'][0]
    def test_each_notification_delivered_with_original_typed_instance(self):
        result=self.run_case(events=['typed','missing','malformed','typed'])
        self.assertEqual([v['sequence'] for v in result['events']], [1,2,3,4])
        self.assertEqual([v['typed'] for v in result['events']], [True,False,False,True])
        self.assertEqual([v['ownInstance'] for v in result['events']], [True,False,False,True])
        self.assertEqual(result['failed'],0)
        self.assertEqual(result['beforeClose'],result['afterClose'])
    def test_nested_sink_preserves_each_sequence(self):
        result=self.run_case(events=['typed','typed'], sink='nested')
        self.assertEqual([v['sequence'] for v in result['events']], [1,2,3])
        self.assertEqual([v['typed'] for v in result['events']], [True,False,True])
    def test_ingress_reentry_does_not_mint_new_sequence_for_old_notification(self):
        result=self.run_case(events=['typed','typed'], ingress='nested')
        self.assertEqual([v['sequence'] for v in result['events']], [2,1,3])
        # A contiguous Policy rejects this order. Sampling the latest sequence
        # after userInfo would incorrectly relabel the original notification.
        self.assertEqual([v['typed'] for v in result['events']], [False,True,True])
    def test_close_during_ingress_revokes_before_native_payload_read_or_sink(self):
        result=self.run_case(events=['typed','typed'], ingress='drop')
        self.assertEqual(result['events'],[])
        self.assertEqual(result['sequence'],1)
    def test_close_from_sink_keeps_receiver_alive_but_no_further_delivery(self):
        result=self.run_case(events=['typed','typed'], sink='drop')
        self.assertEqual(len(result['events']),1)
        self.assertEqual(result['beforeClose'],result['afterClose'])
    def test_ingress_and_sink_panics_are_contained_and_disable_delivery(self):
        for kind in ['ingress','sink']:
            result=self.run_case(events=['typed','typed'], **{kind:'panic'})
            self.assertEqual(result['failed'],1)
            self.assertEqual(result['sequence'],1)
            self.assertEqual(len(result['events']),int(kind=='sink'))
    def test_sequence_exhaustion_never_wraps(self):
        result=self.run_case(events=['typed','typed'], sequence=2**64-2)
        self.assertEqual([v['sequence'] for v in result['events']], [2**64-1,None])
        self.assertEqual(result['sequence'],2**64-1)
if __name__=='__main__':unittest.main(verbosity=2)
