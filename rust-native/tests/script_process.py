#!/usr/bin/env python3
"""Real disposable process groups through the shared core and shipping CLI."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
PROBE = Path(os.environ.get('RUYCAST_SCRIPT_PROBE', ROOT / 'rust-native/target/debug/script-probe'))
CLI = ROOT / '.build/rust/release/ruycast'


class Scripts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT/'.build/full-rust-release/native'))
        cls.shim=Path(os.environ.get('RUYCAST_TEST_SCRIPT_FAULT_SHIM',target/'release/examples/libscript_fault_shim.dylib'))
        if not cls.shim.is_absolute() or not cls.shim.is_file():
            raise RuntimeError('Build --features test-tools --example script-fault-shim and set RUYCAST_TEST_SCRIPT_FAULT_SHIM first')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='ruycast-script-')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.env = dict(os.environ)
        for key in ['DYLD_INSERT_LIBRARIES', 'RUYCAST_HOME', 'RUSTCAST_HOME', 'RUYCAST_CONFIG_DIR', 'RUSTCAST_CONFIG_DIR']:
            self.env.pop(key, None)

    def command(self, arguments, target='/bin/sh', **extra):
        return dict(id='script.fixture', title='Script fixture', kind='script', target=target, arguments=arguments, **extra)

    def run_probe(self, command, fault=None, **options):
        request = dict(command=command, home=str(self.home), **options)
        env = dict(self.env)
        if fault:
            self.trace_path = self.home / 'syscalls.trace'
            env.update(DYLD_INSERT_LIBRARIES=str(self.shim), RUYCAST_SCRIPT_TRACE=str(self.trace_path), RUYCAST_SCRIPT_FAULT=fault)
        output = subprocess.run([str(PROBE)], input=json.dumps(request), capture_output=True, text=True, timeout=5, env=env)
        self.assertEqual(output.returncode, 0, output.stderr)
        result = json.loads(output.stdout)
        self.assertTrue(result['noChildren'])
        return result['result']

    def test_home_expansion_and_null_stdin(self):
        script = self.home / 'fixture'
        script.write_text('#!/bin/sh\nread value; printf "%s" "$?"\n')
        script.chmod(0o700)
        self.assertEqual(self.run_probe(self.command([], '~/fixture'))['response']['result']['stdout'], '1')

    def test_cancel_and_deadline_before_spawn(self):
        sentinel = self.home / 'should-not-run'
        command = self.command(['-c', 'touch "$1"', 'fixture', str(sentinel)])
        for option, end in [('cancelMs', 'cancelled'), ('deadlineMs', 'deadline')]:
            self.assertEqual(self.run_probe(command, **{option: 0})['end'], end)
            self.assertFalse(sentinel.exists())

    def fixture_tree(self):
        sentinel = self.home / 'descendant-ran'
        pid_file = self.home / 'leader.pid'
        # Child ignores TERM and would leave proof if KILL/group cleanup failed.
        child = 'import signal,time,pathlib;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(0.8);pathlib.Path(%r).write_text("bad")' % str(sentinel)
        code = 'import subprocess,sys,time,os,pathlib,signal;pathlib.Path(%r).write_text(str(os.getpid()));signal.signal(signal.SIGTERM,signal.SIG_IGN);subprocess.Popen([sys.executable,"-c",%r]);time.sleep(10)' % (str(pid_file), child)
        return self.command(['-c', code], '/usr/bin/python3'), sentinel, pid_file

    def assert_tree_clean(self, sentinel, pid_file):
        self.assertTrue(pid_file.exists(), 'fixture never started')
        with self.assertRaises(ProcessLookupError):
            os.kill(int(pid_file.read_text()), 0)
        time.sleep(0.85)
        self.assertFalse(sentinel.exists(), 'descendant survived')

    def test_timeout_cleans_group(self):
        command, sentinel, pid_file = self.fixture_tree()
        command['timeout'] = 0.2
        result = self.run_probe(command)
        self.assertEqual(result['end'], 'timeout')
        self.assertIn('timed out', result['response']['error'])
        self.assert_tree_clean(sentinel, pid_file)

    def test_cancellation_cleans_group(self):
        command, sentinel, pid_file = self.fixture_tree()
        result = self.run_probe(command, cancelMs=200)
        self.assertEqual(result['end'], 'cancelled')
        self.assertIn('may have already occurred', result['response']['error'])
        self.assert_tree_clean(sentinel, pid_file)

    def test_absolute_deadline_cleans_group(self):
        command, sentinel, pid_file = self.fixture_tree()
        self.assertEqual(self.run_probe(command, deadlineMs=200)['end'], 'deadline')
        self.assert_tree_clean(sentinel, pid_file)

    def test_callback_panic_cleans_group(self):
        command, sentinel, pid_file = self.fixture_tree()
        self.assertTrue(self.run_probe(command, panicMs=200)['panicCaught'])
        self.assert_tree_clean(sentinel, pid_file)

    def test_early_success_closes_inherited_pipes(self):
        sentinel = self.home / 'pipe-descendant'
        child = 'import time,pathlib;time.sleep(.8);pathlib.Path(%r).write_text("bad")' % str(sentinel)
        code = 'import subprocess,sys;subprocess.Popen([sys.executable,"-c",%r]);print("done")' % child
        result = self.run_probe(self.command(['-c', code], '/usr/bin/python3'))
        self.assertEqual(result['response']['result']['stdout'], 'done\n')
        time.sleep(.85)
        self.assertFalse(sentinel.exists())

    def assert_trace(self, marker):
        lines = [line.split() for line in self.trace_path.read_text().splitlines()]
        self.assertTrue(any(line[0] == marker for line in lines), 'fault was not injected')
        # libc's actual reap must occur after every signal to this group.
        reaped = [(i, int(line[2])) for i, line in enumerate(lines) if line[0] == 'WAIT' and int(line[2]) > 0]
        self.assertEqual(len(reaped), 1, lines)
        index, leader = reaped[0]
        self.assertTrue(any(line[0] == 'KILL' and int(line[1]) == -leader for line in lines[:index]), lines)
        self.assertFalse(any(line[0] == 'KILL' and abs(int(line[1])) == leader for line in lines[index+1:]), lines)
        with self.assertRaises(ProcessLookupError):
            os.kill(leader, 0)



if __name__ == '__main__':
    unittest.main()
