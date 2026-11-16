#!/usr/bin/env python3
"""Actual native owner/callbacks through complete disposable IOKit/Quartz ABI."""
import json
import os
from pathlib import Path
import subprocess
import shutil
import unittest

ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = ROOT / 'target/hid-fixture'
PROBE = Path(os.environ.get('RUYCAST_HID_PROBE', ROOT / 'target/debug/hid-probe'))
HYPER = sum(1 << bit for bit in [17, 18, 19, 20])
CMD = 1 << 20
CTRL = 1 << 18


def op(operation, **values):
    return dict(op=operation, **values)


def plan(hyper=True, **values):
    return op('plan', hyper=hyper, **values)


def raw(usage, down, time=0, device=0, page=7):
    return op('input', usage=usage, down=down, time=time, device=device, page=page)


def event(kind=10, key=0, flags=0, **values):
    return op('event', type=kind, key=key, flags=flags, **values)


class HidSdk(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        DIRECTORY.mkdir(parents=True, exist_ok=True)
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT.parent / '.build/full-rust-release/native'))
        for name, variable, library in [('complete', 'RUYCAST_TEST_HID_SHIM', 'libhid_fixture.dylib'),
                                        ('missing', 'RUYCAST_TEST_HID_PARTIAL', 'libhid_missing_post.dylib')]:
            artifact = Path(os.environ.get(variable, target / 'release/examples' / library))
            if not artifact.is_absolute() or not artifact.is_file():
                raise RuntimeError('Build --features test-tools --example hid-fixture --example hid-missing-post first')
            shutil.copyfile(artifact, DIRECTORY / (name + '.dylib'))

    def run_case(self, operations, original=True, env=0, final_caps=False):
        environment = dict(os.environ)
        environment['DYLD_INSERT_LIBRARIES'] = str(DIRECTORY / 'complete.dylib')
        process = subprocess.run([str(PROBE)], input=json.dumps(dict(operations=operations, original=original, env=env)),
                                 text=True, capture_output=True, env=environment, timeout=10)
        self.assertEqual(process.returncode, 0, process.stderr)
        data = json.loads(process.stdout)
        self.assertEqual(data['cleanup']['allocated'], 0, process.stderr)
        self.assertEqual(data['cleanup']['caps'], int(final_caps), process.stderr)
        return data['rows'], process.stderr

    def test_setup_reentry_stop_at_each_native_boundary_never_reports_on(self):
        for stage in ['tap_create', 'source', 'add_source', 'manager_create', 'device_dictionary', 'device_matching',
                      'input_dictionary', 'input_matching', 'register_input', 'register_remove', 'schedule', 'manager_open', 'tap_enable', 'tap_enabled']:
            rows, _ = self.run_case([plan(), op('hook', name=stage, mode='stop'), op('refresh')], final_caps=True)
            self.assertNotEqual(rows[-1]['status'], 'On', stage)
            self.assertEqual(rows[-1]['state']['allocated'], 0, stage)
            self.assertEqual(rows[-1]['state']['caps'], 1, stage)

    def test_hyper_emission_reentry_does_not_post_stale_down_or_leave_modifiers(self):
        for stage in ['event_source', 'keyboard_event', 'set_type', 'set_flags', 'set_field', 'post']:
            rows, _ = self.run_case([plan(), op('refresh'), op('hook', name=stage, mode='stop'), raw(57, True), op('refresh'), op('stop')])
            self.assertFalse(rows[-1]['state']['lastDown'], stage)
            self.assertEqual(rows[-1]['state']['lastFlags'], 0, stage)
            self.assertEqual(rows[-1]['state']['caps'], 0, stage)

    def test_event_flags_restore_after_setter_reentry_stop(self):
        original = (1 << 63) | CMD | 16
        rows, _ = self.run_case([plan(True, sided={'right-cmd+a': 'side'}), op('refresh'), raw(57, True),
                                 op('hook', name='set_flags', mode='stop'), event(flags=original)])
        self.assertEqual(rows[-1]['result'], dict(consumed=False, flags=original))
        self.assertEqual(rows[-1]['actions'], [])

    def test_lifecycle_gate_panic_preserves_native_ownership_for_retirement(self):
        rows, _ = self.run_case([plan(), op('refresh'), op('panic-gate', value=True), op('refresh'),
                                 op('panic-gate', value=False), op('stop')])
        self.assertIn('panic', rows[3]['result'])
        self.assertTrue(rows[3]['state']['manager'])
        self.assertEqual(rows[5]['state']['allocated'], 0)

    def test_callback_gate_panic_frees_temporary_native_events(self):
        rows, _ = self.run_case([plan(), op('refresh'), op('hook', name='set_field', mode='panic-gate'), raw(57, True),
                                 op('panic-gate', value=False), op('stop')])
        self.assertTrue(rows[3]['failed'])
        self.assertEqual(rows[3]['state']['allocated'], 3)
        self.assertEqual(rows[3]['state']['posts'], 0)
        self.assertEqual(rows[5]['state']['allocated'], 0)

    def test_gate_reentry_with_running_true_cannot_post_or_return_stale_hyper(self):
        rows, _ = self.run_case([plan(), op('refresh'), op('gate-stop', after=2), raw(57, True)])
        self.assertEqual(rows[-1]['state']['posts'], 0)
        self.assertEqual(rows[-1]['state']['allocated'], 0)
        original = (1 << 63) | CMD | 16
        rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), op('gate-stop', after=2), event(flags=original)])
        self.assertEqual(rows[-1]['result'], dict(consumed=False, flags=original))
        self.assertFalse(rows[-1]['state']['lastDown'])

    def test_unmatched_and_nonkey_callbacks_do_not_clone_or_allocate_the_binding_plan(self):
        bindings = {}
        for side in ['left-cmd', 'right-cmd']:
            for modifiers in ['', 'ctrl+', 'alt+', 'shift+', 'ctrl+alt+', 'ctrl+shift+', 'alt+shift+', 'ctrl+alt+shift+']:
                for key in 'abcdefghijklmnopqrstuvwxyz':
                    bindings[modifiers + side + '+' + key] = 'item.' + key
        rows, _ = self.run_case([plan(False, sided=bindings), op('refresh'), op('allocation-events', count=1000)], final_caps=True)
        self.assertEqual(rows[-1]['result'], dict(rustAllocations=0, callbacks=3000))

    def test_missing_or_absent_sdk_table_never_falls_back(self):
        for library in ['missing', None]:
            env = dict(os.environ)
            env.pop('DYLD_INSERT_LIBRARIES', None)
            if library: env['DYLD_INSERT_LIBRARIES'] = str(DIRECTORY / (library + '.dylib'))
            result = subprocess.run([str(PROBE)], input='{}', text=True, capture_output=True, env=env, timeout=10)
            self.assertEqual(result.returncode, 2)
            self.assertIn('Complete HID SDK fixture unavailable.', result.stderr)
            self.assertNotIn('hid-fixture:', result.stderr)
            self.assertNotIn('caps-fixture:', result.stderr)

    def test_disabled_tap_notification_stops_and_refresh_recovers(self):
        for notification in [0xffffffff, 0xfffffffe]:
            rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), event(notification), op('refresh')])
            self.assertNotEqual(rows[3]['status'], 'On')
            self.assertEqual(rows[4]['status'], 'On')
            self.assertFalse(rows[4]['state']['lastDown'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
