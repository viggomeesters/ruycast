#!/usr/bin/env python3
"""Caps ownership through the typed C SDK table, with no actual IOKit calls."""
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = ROOT / 'target/caps-fixture'
PROBE = ROOT / 'target/debug/caps-probe'


def mask(*indexes):
    return sum(1 << i for i in indexes)


class CapsSdk(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIRECTORY.mkdir(parents=True, exist_ok=True)
        for name, flags in [('complete', []), ('missing', ['-DOMIT_SET'])]:
            subprocess.run(['clang', '-std=c11', '-Wall', '-Wextra', '-Werror', '-dynamiclib',
                            *flags, str(ROOT / 'tests/caps_shim.c'), '-o', str(DIRECTORY / (name + '.dylib'))], check=True, capture_output=True)

    def run_case(self, operations, original=True, failures=0, library='complete', nested_at=None):
        env = dict(os.environ)
        env['DYLD_INSERT_LIBRARIES'] = str(DIRECTORY / (library + '.dylib'))
        result = subprocess.run([str(PROBE)], input=json.dumps(dict(operations=operations, original=original, failures=failures, nestedAt=nested_at)),
                                text=True, capture_output=True, env=env, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = [line.removeprefix('caps-fixture:') for line in result.stderr.splitlines() if line.startswith('caps-fixture:')]
        return json.loads(result.stdout), trace

    def test_roundtrip_both_original_latches_and_idempotent_refresh(self):
        for original in [False, True]:
            data, trace = self.run_case(['activate', 'activate', 'clear', 'retire', 'retire'], original)
            self.assertEqual(trace, ['matching', 'service', 'task', 'open', 'release', 'get', 'set:false', 'set:false',
                                     'set:true' if original else 'set:false', 'close'])
            self.assertEqual(data['finalState'], int(original))
            self.assertFalse(data['recoveryPending'])
            self.assertEqual([v['result'] for v in data['results']], ['Ok(())'] * 5)

    def test_acquisition_failure_releases_service_and_never_uses_failed_output(self):
        for stage, expected, tail in [(1, 'Open(-1)', ['matching']), (2, 'Open(-2)', ['matching', 'service']),
                                      (4, 'Open(-4)', ['matching', 'service', 'task', 'open', 'release']),
                                      (5, 'Open(-5)', ['matching', 'service', 'task', 'open', 'release', 'close'])]:
            data, trace = self.run_case(['activate'], failures=mask(stage))
            self.assertEqual(trace, tail)
            self.assertEqual(data['results'][0]['result'], 'Err(' + expected + ')')
            self.assertEqual(data['finalState'], 1)

    def test_failed_read_never_guesses_original_or_mutates(self):
        data, trace = self.run_case(['activate'], failures=mask(6))
        self.assertEqual(trace, ['matching', 'service', 'task', 'open', 'release', 'get', 'close'])
        self.assertEqual(data['results'][0]['result'], 'Err(Read(-6))')
        self.assertEqual(data['finalState'], 1)

    def test_failed_clear_restores_even_when_failed_call_changed_latch(self):
        data, trace = self.run_case(['activate'], failures=mask(7))
        self.assertEqual(trace[-3:], ['set:false', 'set:true', 'close'])
        self.assertEqual(data['results'][0]['result'], 'Err(Clear(-7))')
        self.assertEqual(data['finalState'], 1)

    def test_restore_failure_survives_drop_blocks_reopen_and_recovers(self):
        data, trace = self.run_case(['activate', 'drop', 'new', 'activate', 'recover', 'activate', 'retire'], failures=mask(8))
        rows = data['results']
        self.assertTrue(rows[1]['recoveryPending'])
        self.assertEqual(rows[1]['state'], 3)  # original true, still-open handle
        self.assertEqual(rows[3]['result'], 'Err(Open(-4))')
        self.assertEqual(rows[4]['result'], 'Ok(())')
        self.assertFalse(rows[4]['recoveryPending'])
        self.assertEqual(rows[4]['state'], 1)
        self.assertEqual(trace[7:10], ['set:true', 'set:true', 'close'])
        self.assertEqual(data['finalState'], 1)
        self.assertFalse(data['recoveryPending'])

    def test_repeated_failed_quarantine_restore_retains_authority(self):
        data, trace = self.run_case(['activate', 'drop', 'recover', 'recover'], failures=mask(8, 9))
        self.assertEqual(data['results'][2]['result'], 'Err(Restore(-9))')
        self.assertTrue(data['results'][2]['recoveryPending'])
        self.assertEqual(trace[-4:], ['set:true', 'set:true', 'set:true', 'close'])
        self.assertFalse(data['recoveryPending'])

    def test_close_error_consumes_handle_without_double_close(self):
        data, trace = self.run_case(['activate', 'retire', 'retire'], failures=mask(9))
        self.assertEqual(data['results'][1]['result'], 'Err(Close(-9))')
        self.assertEqual(trace.count('close'), 1)
        self.assertEqual(data['finalState'], 1)

    def test_nested_second_owner_cannot_acquire_during_setup_clear_restore_or_close(self):
        for stage in [1, 2, 3, 4, 5, 6, 7, 8, 9]:
            data, trace = self.run_case(['activate', 'retire'], nested_at=stage)
            self.assertEqual(data['nested'], 'Ok(Err(Open(-5)))')
            self.assertEqual(trace.count('matching'), 1)
            self.assertEqual(trace.count('open'), 1)
            self.assertEqual(trace.count('close'), 1)
            self.assertEqual(data['finalState'], 1)

    def test_incomplete_or_absent_fixture_fails_before_any_sdk_entry(self):
        for library in ['missing', None]:
            env = dict(os.environ)
            env.pop('DYLD_INSERT_LIBRARIES', None)
            if library:
                env['DYLD_INSERT_LIBRARIES'] = str(DIRECTORY / (library + '.dylib'))
            result = subprocess.run([str(PROBE)], input='{}', text=True, capture_output=True, env=env, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn('Complete Caps SDK fixture unavailable.', result.stderr)
            self.assertNotIn('caps-fixture:', result.stderr)


if __name__ == '__main__':
    unittest.main()
