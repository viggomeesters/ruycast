#!/usr/bin/env python3
"""Actual native owner/callbacks through complete disposable IOKit/Quartz ABI."""
import json
import os
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
DIRECTORY = ROOT / 'target/hid-fixture'
PROBE = ROOT / 'target/debug/hid-probe'
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
        for name, flags in [('complete', []), ('missing', ['-DOMIT_HID_POST'])]:
            subprocess.run(['clang', '-std=c11', '-Wall', '-Wextra', '-Werror', '-dynamiclib', *flags,
                            str(ROOT / 'tests/hid_shim.c'), '-o', str(DIRECTORY / (name + '.dylib'))],
                           check=True, capture_output=True)

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

    def test_hyper_physical_down_up_flags_and_balanced_stop(self):
        rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), event(flags=(1 << 63) | (1 << 16) | 0x10), raw(57, False), op('stop')])
        self.assertEqual(rows[1]['status'], 'On')
        self.assertTrue(rows[2]['state']['lastDown'])
        self.assertEqual(rows[2]['state']['lastFlags'], HYPER)
        self.assertEqual(rows[3]['result'], dict(consumed=False, flags=(1 << 63) | 0x10 | HYPER))
        self.assertEqual(rows[4]['state']['lastFlags'], 0)
        self.assertEqual(rows[5]['state']['caps'], 0)

    def test_caps_on_two_devices_and_removal_releases_only_last(self):
        rows, _ = self.run_case([plan(), op('refresh'), raw(57, True, device=0), raw(57, True, device=1),
                                 op('remove', device=0), event(), op('remove', device=1), event()])
        self.assertEqual(rows[5]['result']['flags'], HYPER)
        self.assertEqual(rows[7]['result']['flags'], 0)
        self.assertEqual(rows[6]['state']['lastDown'], False)

    def test_sided_repeat_release_and_backpressure_do_not_eat_input(self):
        rows, _ = self.run_case([plan(False, sided={'right-cmd+a': 'item'}), op('refresh'),
                                 op('accept', value=False), event(flags=CMD | 16), event(11, flags=CMD | 16),
                                 op('accept', value=True), event(flags=CMD | 16), event(flags=CMD | 16, repeat=True),
                                 event(11, flags=CMD | 16), event(11, flags=CMD | 16), event(flags=CMD | 8)], final_caps=True)
        self.assertFalse(rows[3]['result']['consumed'])
        self.assertFalse(rows[4]['result']['consumed'])
        for index in [6, 7, 8]: self.assertTrue(rows[index]['result']['consumed'])
        for index in [9, 10]: self.assertFalse(rows[index]['result']['consumed'])
        self.assertEqual(rows[-1]['actions'], [dict(generation=1, canonical='right-cmd+a', action='item')])

    def test_recorder_episode_without_callbacks_invalidates_taps_and_swallowed_keys(self):
        rows, _ = self.run_case([plan(True, modifiers={'224': 'tap'}, sided={'right-cmd+a': 'side'}), op('refresh'),
                                 raw(224, True), op('gate', epoch=1, recording=True), op('gate', epoch=2), raw(224, False, .2),
                                 event(flags=CMD | 16), op('gate', epoch=3, recording=True), event(11, flags=CMD | 16),
                                 raw(57, True), event(flags=CMD | 16)])
        self.assertEqual(rows[5]['actions'], [])
        self.assertFalse(rows[8]['result']['consumed'])
        self.assertFalse(rows[10]['result']['consumed'])
        self.assertEqual(rows[10]['result']['flags'], HYPER | 16)

    def test_scroll_and_mouse_cancel_modifier_taps_with_native_types(self):
        for kind in [1, 3, 10, 22, 25]:
            rows, _ = self.run_case([plan(False, modifiers={'224': 'tap'}), op('refresh'), raw(224, True),
                                     event(kind), raw(224, False, .2)], final_caps=True)
            self.assertEqual(rows[-1]['actions'], [], kind)
        rows, _ = self.run_case([plan(False, modifiers={'224': 'tap'}), op('refresh'), raw(224, True), raw(224, False, .5)], final_caps=True)
        self.assertEqual(rows[-1]['actions'], [dict(generation=1, canonical='ctrl+left-control', action='tap')])

    def test_identical_plan_generation_preserves_held_physical_and_hardware(self):
        rows, trace = self.run_case([plan(), op('refresh'), raw(57, True), plan(generation=2), op('refresh'), event()])
        self.assertEqual(rows[-1]['result']['flags'], HYPER)
        self.assertEqual(trace.count('hid-fixture:manager_create\n'), 1)

    def test_partial_setup_failure_always_restores_original_and_can_retry(self):
        for stage in ['tap_create', 'source', 'manager_create', 'device_dictionary', 'input_dictionary', 'manager_open', 'tap_enable']:
            for original in [False, True]:
                rows, _ = self.run_case([plan(), op('failure', name=stage), op('refresh'), op('failure', name=''), op('refresh')], original)
                self.assertNotEqual(rows[2]['status'], 'On', stage)
                self.assertEqual(rows[2]['state']['caps'], int(original), stage)
                self.assertEqual(rows[2]['state']['allocated'], 0, stage)
                self.assertEqual(rows[4]['status'], 'On', stage)

    def test_failed_manager_or_tap_retirement_survives_drop_and_blocks_new_setup(self):
        for stage in ['manager_close', 'invalidate']:
            rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), op('failure', name=stage), op('drop'),
                                     op('new'), plan(), op('refresh'), op('failure', name=''), op('recover'), op('refresh')])
            self.assertNotEqual(rows[7]['status'], 'On')
            self.assertTrue(rows[9]['result'])
            self.assertEqual(rows[10]['status'], 'On')

    def test_failed_balancing_creation_retries_even_when_features_are_off(self):
        rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), op('failure', name='event_source'), plan(False),
                                 op('refresh'), op('failure', name=''), op('refresh')])
        self.assertIn('cleanup interrupted', rows[5]['status'])
        self.assertEqual(rows[7]['status'], 'Off')
        self.assertFalse(rows[7]['state']['lastDown'])
        self.assertEqual(rows[7]['state']['lastFlags'], 0)

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

    def test_generated_events_pass_unmodified_and_secure_input_recovers(self):
        rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), event(flags=1 << 16, tag=0x5255535443415354),
                                 op('env', value=1), event(flags=CMD | 16), op('refresh'), op('env', value=0), op('refresh')])
        self.assertEqual(rows[3]['result']['flags'], 1 << 16)
        self.assertEqual(rows[5]['result']['flags'], CMD | 16)
        self.assertEqual(rows[6]['status'], 'Paused during secure input')
        self.assertEqual(rows[8]['status'], 'On')

    def test_off_sleep_and_environment_precedence_short_circuit_discovery(self):
        rows, trace = self.run_case([plan(False), op('refresh')], env=15, final_caps=True)
        self.assertEqual(rows[-1]['status'], 'Off')
        self.assertNotIn('hid-fixture:raycast', trace)
        for env, expected in [(15, 'Quit Raycast'), (7, 'Paused during secure'), (6, 'Allow Accessibility'), (4, 'Allow Input Monitoring')]:
            rows, _ = self.run_case([plan(), op('refresh')], env=env, final_caps=True)
            self.assertIn(expected, rows[-1]['status'])
        rows, trace = self.run_case([plan(), op('sleep', value=True), op('refresh')], final_caps=True)
        self.assertEqual(rows[-1]['status'], 'Paused')
        self.assertNotIn('hid-fixture:raycast', trace)

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

    def test_exact_canonical_route_is_preserved_when_two_bindings_share_an_id(self):
        rows, _ = self.run_case([plan(False, sided={'right-cmd+a': 'same', 'left-cmd+a': 'same'}), op('refresh'),
                                 event(flags=CMD | 16), event(11, flags=CMD | 16), event(flags=CMD | 8)], final_caps=True)
        self.assertEqual([a['canonical'] for a in rows[-1]['actions']], ['right-cmd+a', 'left-cmd+a'])

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

    def test_caps_restore_failure_retains_authority_and_recovers_after_features_off(self):
        rows, _ = self.run_case([plan(), op('caps-failures', value=sum(1 << n for n in range(8, 30))),
                                 op('failure', name='tap_create'), op('refresh'), op('drop'), op('new'), plan(False),
                                 op('refresh'), op('caps-failures', value=0), op('failure', name=''), op('recover'), op('refresh')], final_caps=True)
        self.assertIn('cleanup interrupted', rows[3]['status'])
        self.assertEqual(rows[3]['state']['caps'], 3)
        self.assertNotEqual(rows[7]['status'], 'On')
        self.assertTrue(rows[10]['result'])
        self.assertEqual(rows[11]['status'], 'Off')
        self.assertEqual(rows[11]['state']['caps'], 1)

    def test_disabled_tap_notification_stops_and_refresh_recovers(self):
        for notification in [0xffffffff, 0xfffffffe]:
            rows, _ = self.run_case([plan(), op('refresh'), raw(57, True), event(notification), op('refresh')])
            self.assertNotEqual(rows[3]['status'], 'On')
            self.assertEqual(rows[4]['status'], 'On')
            self.assertFalse(rows[4]['state']['lastDown'])


if __name__ == '__main__':
    unittest.main(verbosity=2)
