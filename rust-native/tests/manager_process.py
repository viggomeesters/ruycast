#!/usr/bin/env python3
"""Own blocked SDK calls prove real manager retirement/capacity boundaries."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
import workspace_process as workspace_fixture

ROOT, SHIM = workspace_fixture.ROOT, workspace_fixture.SHIM

BIN = ROOT / 'target/debug/manager-probe'


class ExecutionManager(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workspace_fixture.Workspace.setUpClass()

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='manager-', dir=ROOT / 'target')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve()
        self.home.chmod(0o700)
        (self.home / '.ruycast-test-fixture').write_text('own-disposable-fixture-v1')

    def probe(self, scenario, *, shim=True):
        env = dict(os.environ)
        for key in list(env):
            if key.startswith(('RUYCAST_', 'RUSTCAST_')) or key == 'DYLD_INSERT_LIBRARIES':
                env.pop(key)
        env.update(HOME=str(self.home), RUYCAST_WORKSPACE_FIXTURE_HOME=str(self.home))
        if shim:
            env['DYLD_INSERT_LIBRARIES'] = str(SHIM)
        run = subprocess.run([str(BIN)], input=json.dumps(dict(scenario=scenario)), text=True,
                             capture_output=True, env=env, timeout=10)
        result = json.loads(run.stdout)
        if not shim:
            self.assertEqual(run.returncode, 1)
            self.assertFalse(result['ok'])
            self.assertIn('boundary', result['error'])
            return result
        self.assertEqual(run.returncode, 0, run.stderr + run.stdout)
        self.assertTrue(result['ok'])
        self.assertFalse(result['ownApplicationActive'])
        self.assertEqual(result['ownWindowCount'], 0)
        path = self.home / 'workspace-trace.jsonl'
        self.trace = [json.loads(s) for s in path.read_text().splitlines()]
        self.assertEqual(self.trace[0]['selectors'], 4)
        evidence = os.environ.get('RUYCAST_NATIVE_EVIDENCE_DIR')
        if evidence:
            dest = Path(evidence) / 'manager'
            dest.mkdir(parents=True, exist_ok=True)
            (dest / (scenario+'.json')).write_text(json.dumps(dict(result=result, trace=self.trace), indent=2))
        return result

    def no_open(self):
        self.assertFalse(any(x['phase'].startswith('open-') for x in self.trace))

    def test_sdk_lookup_retirement_returns_before_release_and_revokes_open(self):
        data = self.probe('retire-lookup')
        result = data['result']
        outcome = result['deliveries'][0]['outcome']
        self.assertFalse(outcome['committed'])
        self.assertFalse(outcome['response']['ok'])
        self.assertEqual(result['beforeRelease'], 1)
        self.assertEqual(result['replacementBeforeRelease'], 1)
        self.assertTrue(result['releasedOnlyAfterShutdown'])
        self.assertEqual(result['afterRelease'], 0)
        self.assertEqual(result['secondDelivery'], [])
        self.assertEqual(data['beforeCommitCalls'], 0)
        self.no_open()

    def test_committed_blocked_default_open_returns_unknown_without_waiting(self):
        data = self.probe('retire-sync')
        result = data['result']
        outcome = result['deliveries'][0]['outcome']
        self.assertTrue(outcome['committed'])
        self.assertEqual(outcome['kind'], 'workspace')
        self.assertFalse(outcome['captureAvailable'])
        self.assertEqual(outcome['response']['code'], 'open-outcome-unknown')
        self.assertTrue(outcome['response']['actionCompleted'])
        self.assertEqual(result['beforeRelease'], 1)
        self.assertEqual(result['replacementBeforeRelease'], 1)
        self.assertEqual(result['afterRelease'], 0)
        self.assertEqual(result['secondDelivery'], [])
        self.assertEqual([x['phase'] for x in self.trace], ['installed','open-default'])

    def test_owner_hook_permission_is_not_sdk_commit(self):
        data = self.probe('retire-hook')
        result = data['result']
        self.assertEqual(data['beforeCommitCalls'], 1)
        self.assertFalse(result['deliveries'][0]['outcome']['committed'])
        self.assertEqual(result['beforeRelease'], 1)
        self.assertEqual(result['afterRelease'], 0)
        self.assertEqual(result['secondDelivery'], [])
        self.no_open()

    def test_process_wide_cli_reservation_survives_manager_replacement(self):
        data = self.probe('capacity')
        result = data['result']
        self.assertTrue(result['cliRejected'])
        self.assertTrue(result['nativeReserved'])
        self.assertTrue(result['nativeRejected'])
        self.assertEqual(result['beforeRelease'], result['limit'])
        self.assertGreaterEqual(result['afterShutdown'], 1)
        self.assertEqual(result['afterRelease'], 0)
        self.assertEqual(len(result['oldDeliveries']), 1)
        self.assertEqual(len(result['deliveries']), result['limit']-1)
        self.assertEqual(data['beforeCommitCalls'], 0)
        self.no_open()

    def test_ready_success_wins_shutdown_and_delivers_exactly_once(self):
        data = self.probe('ready')
        result = data['result']
        self.assertEqual(result['deliveries'][0]['context'], 7)
        outcome = result['deliveries'][0]['outcome']
        self.assertEqual(outcome['kind'], 'workspace')
        self.assertTrue(outcome['committed'])
        self.assertEqual(outcome['response'], dict(ok=True, result=dict(command='fixture',state='opened')))
        self.assertFalse(outcome['captureAvailable'])
        self.assertEqual(result['secondDelivery'], [])
        self.assertEqual(result['liveWorkers'], 0)
        self.assertEqual(data['pendingCallbacks'], 0)

    def test_fixture_without_interposition_fails_before_any_sdk_access(self):
        self.probe('ready', shim=False)
        self.assertFalse((self.home / 'workspace-trace.jsonl').exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
