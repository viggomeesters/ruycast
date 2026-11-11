"""Rust host + unchanged legacy CLI, isolated profiles and real Unix sockets."""
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
HOST = Path(os.environ.get('RUYCAST_TEST_APP_HOST', ROOT / 'target/headless-tools/debug/ruycast-native'))
CLI = ROOT.parent / '.build/rust/release/ruycast'
DEFAULT = json.loads((ROOT.parent / 'config/default.json').read_text())


class ClosedResponse(AssertionError):
    """RPC peer closed before sending a reply; never a readiness success."""


class Host:
    def __init__(self, start=True):
        self.temp = tempfile.TemporaryDirectory(prefix='rpc-', dir=Path(os.environ.get('RUYCAST_NATIVE_TEST_FIXTURE_DIR', ROOT / 'target')))
        self.base = Path(self.temp.name).resolve()
        self.profile = self.base / 'p'
        self.profile.mkdir(mode=0o700)
        self.config = self.profile / 'config.json'
        self.config.write_text(json.dumps(DEFAULT))
        (self.base / '.ruycast-test-fixture').write_text('own-disposable-fixture-v1')
        self.applications = self.base / 'Applications'
        self.applications.mkdir()
        self.env = dict(os.environ)
        for name in ['RUYCAST_NATIVE_TEST_CONFIG_ADOPTION_BARRIER','RUYCAST_NATIVE_TEST_KEYBOARD','RUYCAST_KEYBOARD_FIXTURE_HOME','RUYCAST_NATIVE_TEST_SNIPPET','RUYCAST_HOME', 'RUSTCAST_HOME', 'RUYCAST_CONFIG_DIR', 'RUSTCAST_CONFIG_DIR', 'DYLD_INSERT_LIBRARIES', 'RUYCAST_NATIVE_TEST_APP_ROOTS', 'RUYCAST_NATIVE_TEST_APP_BARRIER', 'RUYCAST_WORKSPACE_FIXTURE_HOME', 'RUYCAST_NATIVE_TEST_DISPATCH_MS', 'RUYCAST_NATIVE_TEST_CLIPBOARD', 'RUYCAST_NATIVE_TEST_CLIPBOARD_TEXT', 'RUYCAST_NATIVE_TEST_CLIPBOARD_GUARD', 'RUYCAST_NATIVE_TEST_REQUIRE_HIDE', 'RUYCAST_NATIVE_TEST_TEMPLATE_BARRIER','RUYCAST_NATIVE_TEST_OPEN_DEADLINE_MS']:
            self.env.pop(name, None)
        self.env.update(RUYCAST_HOME=str(self.profile), HOME=str(self.base), RUYCAST_APP_BINARY=str(HOST), RUYCAST_NATIVE_TEST_APP_ROOTS=json.dumps([str(self.applications)]))
        self.process = None
        if start:
            try:
                self.start()
            except BaseException:
                self.close()
                raise

    def start(self):
        self.process = subprocess.Popen([str(HOST), '--headless-test'], env=self.env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        end = time.monotonic() + 6
        while time.monotonic() < end:
            if self.process.poll() is not None:
                out, err = self.process.communicate()
                raise AssertionError('Own Rust host exited: ' + out.decode() + err.decode())
            try:
                if self.rpc({'op': 'status'}, timeout=.2).get('ok'):
                    return
            except (OSError, ValueError, ClosedResponse):
                pass
            time.sleep(.02)
        raise AssertionError('Own Rust host did not become ready')

    def connect(self, timeout=3):
        stream = socket.socket(socket.AF_UNIX)
        stream.settimeout(timeout)
        try:
            stream.connect(str(self.profile / 'control.sock'))
        except BaseException:
            stream.close()
            raise
        return stream

    def rpc(self, request, timeout=3):
        with self.connect(timeout) as stream:
            stream.sendall(json.dumps(request).encode() + b'\n')
            with stream.makefile('rb') as reader:
                line = reader.readline()
                if not line:
                    raise ClosedResponse(f'Own host closed without response: op={request.get("op")} action={request.get("action")} exit={self.process.poll() if self.process else None}')
                return json.loads(line)

    def cli(self, *args):
        run = subprocess.run([str(CLI), *args], env=self.env, capture_output=True, text=True, timeout=6)
        if run.returncode != 0:
            raise AssertionError(run.stdout + run.stderr)
        return json.loads(run.stdout)

    def wait(self, predicate, timeout=7):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            result = self.rpc({'op': 'status'})['result']
            if predicate(result):
                return result
            time.sleep(.03)
        raise AssertionError('Observed configuration did not reach expected state')

    def replace(self, value, path=None):
        path = path or self.config
        temporary = path.with_suffix('.candidate')
        temporary.write_text(json.dumps(value) if not isinstance(value, bytes) else value.decode())
        os.replace(temporary, path)

    def stop(self, graceful=True):
        if self.process and self.process.poll() is None:
            if graceful:
                try:
                    self.rpc({'op': 'quit'})
                except (OSError, ValueError):
                    self.process.kill()
            else:
                self.process.kill()
        if self.process:
            try:
                out, err = self.process.communicate(timeout=5)
                if graceful and self.process.returncode:
                    raise AssertionError(f'Own host failed ({self.process.returncode}): '+out.decode()+err.decode())
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.communicate(timeout=2)
        self.process = None

    def close(self):
        try:
            self.stop()
        finally:
            self.temp.cleanup()


class RuntimeProcesses(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.writer_fixture = tempfile.TemporaryDirectory(prefix='config-writer-sdk-', dir=Path(os.environ.get('RUYCAST_NATIVE_TEST_FIXTURE_DIR', ROOT / 'target')))
        cls.writer_shim = Path(cls.writer_fixture.name) / 'sync.dylib'
        target=Path(os.environ.get('CARGO_TARGET_DIR',ROOT.parent/'.build/full-rust-release/native'))
        cls.writer_shim=Path(os.environ.get('RUYCAST_TEST_FCNTL_SHIM',target/'release/examples/libfcntl_sync_shim.dylib'))
        if not cls.writer_shim.is_absolute() or not cls.writer_shim.is_file():
            raise RuntimeError('Build --features test-tools --example fcntl-sync-shim and set RUYCAST_TEST_FCNTL_SHIM first')

    @classmethod
    def tearDownClass(cls):
        cls.writer_fixture.cleanup()

    def writer_host(self, delayed=False, mutation=False):
        host = self.host(start=False)
        marker = host.base / 'writer-entered'
        host.env.update(DYLD_INSERT_LIBRARIES=str(self.writer_shim), RUYCAST_TEST_FSYNC_DIRECTORY=str(host.profile), RUYCAST_TEST_FSYNC_MARKER=str(marker))
        if delayed:
            host.env.update(RUYCAST_TEST_FSYNC_DELAY_MS='750', RUYCAST_TEST_FSYNC_DELAY_ONLY='1')
        if mutation:
            host.env.update(RUYCAST_TEST_FSYNC_MUTATE='selector', RUYCAST_TEST_SELECTOR_JSON=json.dumps({'directory':str(host.profile)}))
        host.start()
        self.assertFalse(marker.exists())
        return host, marker

    def wait_marker(self, marker):
        end = time.monotonic() + 3
        while not marker.exists():
            self.assertLess(time.monotonic(), end, 'actual directory sync not reached')
            time.sleep(.002)

    def test_held_configuration_write_keeps_status_search_responsive_and_second_writer_bounded(self):
        host, marker = self.writer_host(delayed=True)
        result = []
        worker = threading.Thread(target=lambda:result.append(host.rpc({'op':'config.directory','directory':str(host.base/'portable')})))
        worker.start()
        try:
            self.wait_marker(marker)
            at = time.monotonic()
            status = host.rpc({'op':'status'})['result']
            self.assertTrue(status['configurationUpdating'])
            self.assertTrue(host.rpc({'op':'search','query':'finder'})['ok'])
            rejected = host.rpc({'op':'config.directory','directory':str(host.base/'second')})
            self.assertFalse(rejected['ok']); self.assertIn('updated', rejected['error'])
            self.assertLess(time.monotonic()-at, .5)
        finally:
            worker.join(timeout=4)
        self.assertFalse(worker.is_alive()); self.assertEqual(len(result), 1)
        self.assertTrue(result[0]['ok']); self.assertTrue(result[0]['configurationWrite']['published'])
        self.assertFalse(result[0]['configurationWrite']['outcomeUncertain'])
        self.assertFalse((host.base/'second'/'config.json').exists())
        host.wait(lambda s:not s['configurationUpdating'])

    def test_quit_while_filesystem_write_is_entered_returns_conservative_configuration_truth(self):
        host, marker = self.writer_host(delayed=True)
        with host.connect() as pending:
            pending.sendall(json.dumps({'op':'config.directory','directory':str(host.base/'portable')}).encode()+b'\n')
            self.wait_marker(marker)
            at = time.monotonic(); self.assertTrue(host.rpc({'op':'quit'})['ok'])
            with pending.makefile('rb') as reader:
                response = json.loads(reader.readline())
            self.assertLess(time.monotonic()-at, .5)
            self.assertFalse(response['ok'])
            self.assertEqual(response['configurationWrite'], {'published':False,'outcomeUncertain':True})
            self.assertIn('may have been saved', response['error'])
        host.process.communicate(timeout=4)
        self.assertEqual(host.process.returncode, 0)
        self.assertTrue((host.base/'portable'/'config.json').exists())
        self.assertTrue((host.profile/'config-location.json').exists())

    def test_postpublication_selector_conflict_reports_saved_outcome_and_preserves_both_copies(self):
        host, marker = self.writer_host(mutation=True)
        before = host.config.read_bytes()
        response = host.rpc({'op':'config.directory','directory':str(host.base/'portable')})
        self.assertTrue(marker.exists()); self.assertFalse(response['ok'])
        self.assertEqual(response['configurationWrite'], {'published':True,'outcomeUncertain':False})
        self.assertTrue(response['configurationReconciliationRequired'])
        self.assertIn('may have been saved', response['error'])
        self.assertEqual(host.config.read_bytes(), before)
        self.assertEqual((host.base/'portable'/'config.json').read_bytes(), before)
        host.wait(lambda s:not s['configurationUpdating'])

    def host(self, start=True):
        host = Host(start)
        self.addCleanup(host.close)
        return host

    def test_empty_or_missing_explicit_profile_never_modifies_cwd(self):
        with tempfile.TemporaryDirectory(prefix='entry-', dir=Path(os.environ.get('RUYCAST_NATIVE_TEST_FIXTURE_DIR', ROOT / 'target'))) as directory:
            cwd = Path(directory)
            cwd.chmod(0o755)
            for name, value in [(None, None), ('RUYCAST_HOME', ''), ('RUSTCAST_HOME', '')]:
                env = dict(os.environ)
                for key in ['RUYCAST_HOME', 'RUSTCAST_HOME', 'RUYCAST_CONFIG_DIR']:
                    env.pop(key, None)
                if name:
                    env[name] = value
                run = subprocess.run([str(HOST), '--headless-test'], cwd=cwd, env=env, capture_output=True, timeout=3)
                self.assertNotEqual(run.returncode, 0)
                self.assertEqual(stat.S_IMODE(cwd.stat().st_mode), 0o755)
                self.assertFalse(list(cwd.iterdir()))

    def test_legacy_cli_status_search_reload_and_config_edit(self):
        host = self.host()
        status = host.cli('status')['result']
        self.assertEqual(status['implementation'], 'rust')
        self.assertFalse(status['productionReady'])
        self.assertTrue(status['headless'])
        self.assertIn(status['health'], ('healthy', 'needs-attention'))
        self.assertEqual(host.rpc({'op': 'search', 'query': 'finder'})['result'][0]['id'], 'app.finder')
        result = host.cli('config', 'set', 'launcherHotkey', json.dumps('cmd+space'))
        self.assertTrue(result['ok'])
        self.assertEqual(json.loads(host.config.read_text())['launcherHotkey'], 'cmd+space')
        self.assertTrue(host.cli('reload')['ok'])
        self.assertTrue(host.rpc({'op': 'execute', 'id': 'finder', 'dryRun': True})['dryRun'])
        self.assertFalse(host.rpc({'op': 'execute', 'id': 'finder'})['ok'])

    def test_raw_nullable_or_invalid_command_arrays_preserve_last_good(self):
        host=self.host();value=json.loads(json.dumps(DEFAULT))
        for field,invalid in [('aliases',None),('arguments',None),('aliases','bad'),('arguments',[None]),('aliases',{})]:
            bad=json.loads(json.dumps(value));bad['commands'][0][field]=invalid;host.replace(bad);self.assertFalse(host.rpc({'op':'reload'})['ok']);self.assertEqual(host.rpc({'op':'status'})['result']['commands'],len(value['commands']))
            cli=subprocess.run([str(CLI),'config','validate',str(host.config)],env=host.env,capture_output=True,text=True,timeout=6);self.assertNotEqual(cli.returncode,0);self.assertFalse(json.loads(cli.stdout)['ok'])
            host.replace(value);self.assertTrue(host.rpc({'op':'reload'})['ok'])
        self.assertTrue(host.cli('config','validate',str(host.config))['ok'])
    def test_atomic_replace_invalid_last_good_and_recovery(self):
        host = self.host()
        changed = json.loads(json.dumps(DEFAULT))
        changed['commands'].append({'id': 'fixture.extra', 'title': 'Extra', 'kind': 'url', 'target': 'https://example.com', 'aliases': []})
        host.replace(changed)
        host.wait(lambda s: s['commands'] == len(changed['commands']))
        host.replace(b'broken PRIVATE-SENTINEL')
        status = host.wait(lambda s: s['configError'] is not None)
        self.assertEqual(status['commands'], len(changed['commands']))
        self.assertNotIn('PRIVATE-SENTINEL', str(status))
        self.assertEqual(host.config.read_bytes(), b'broken PRIVATE-SENTINEL')
        host.replace(DEFAULT)
        host.wait(lambda s: s['configError'] is None and s['commands'] == len(DEFAULT['commands']))

    def test_in_place_edit_is_seen_by_five_second_fallback(self):
        host = self.host()
        value = json.loads(json.dumps(DEFAULT))
        value['theme'] = 'dark'
        host.config.write_text(json.dumps(value))
        host.wait(lambda s: s['theme'] == 'dark')

    def test_external_selector_rebinds_observation_to_portable_directory(self):
        host = self.host()
        portable = host.base / 'portable'
        portable.mkdir(mode=0o700)
        (portable / 'config.json').write_text(json.dumps(DEFAULT))
        host.replace({'directory': str(portable)}, host.profile / 'config-location.json')
        host.wait(lambda s: s['configPath'] == str(portable / 'config.json'))
        changed = json.loads(json.dumps(DEFAULT))
        changed['theme'] = 'light'
        host.replace(changed, portable / 'config.json')
        host.wait(lambda s: s['theme'] == 'light')
        self.assertEqual(json.loads(host.config.read_text()), DEFAULT)

    def test_selector_size_bound_and_recovery_keep_valid_state(self):
        host = self.host()
        host.replace(b' ' * 1_000_001, host.profile / 'config-location.json')
        status = host.wait(lambda s: s['configError'] is not None)
        self.assertEqual(status['commands'], len(DEFAULT['commands']))
        host.replace({'directory': str(host.profile)}, host.profile / 'config-location.json')
        host.wait(lambda s: s['configError'] is None)

    def test_relocate_keeps_original_then_cli_edits_new_target(self):
        host = self.host()
        original = host.config.read_bytes()
        destination = host.base / 'portable'
        result = host.rpc({'op': 'config.directory', 'directory': str(destination)})
        self.assertTrue(result['ok'])
        self.assertTrue(result['result']['originalRetained'])
        self.assertEqual(host.config.read_bytes(), original)
        self.assertTrue(host.cli('config', 'set', 'clipboard.limit', '51')['ok'])
        self.assertEqual(json.loads((destination / 'config.json').read_text())['clipboard']['limit'], 51)
        self.assertEqual(host.config.read_bytes(), original)

    def test_slow_incomplete_request_has_absolute_read_deadline(self):
        host = self.host()
        with host.connect(timeout=5) as stream:
            def send():
                for _ in range(20):
                    try:
                        stream.sendall(b' ')
                    except OSError:
                        return
                    time.sleep(.2)
            worker = threading.Thread(target=send, daemon=True)
            start = time.monotonic()
            worker.start()
            with stream.makefile('rb') as reader:
                self.assertFalse(json.loads(reader.readline())['ok'])
            self.assertLess(time.monotonic() - start, 5)
        worker.join(timeout=1)
        self.assertTrue(host.rpc({'op': 'status'})['ok'])

    def test_second_instance_refused_and_crashed_socket_recovers(self):
        host = self.host()
        second = subprocess.run([str(HOST), '--headless-test'], env=host.env, capture_output=True, timeout=3)
        self.assertNotEqual(second.returncode, 0)
        self.assertTrue(host.rpc({'op': 'status'})['ok'])
        host.stop(graceful=False)
        self.assertTrue((host.profile / 'control.sock').exists())
        host.start()
        self.assertEqual(stat.S_IMODE(host.profile.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((host.profile / 'control.sock').stat().st_mode), 0o600)
        host.stop()
        self.assertFalse((host.profile / 'control.sock').exists())

    def test_invalid_initial_config_releases_instance_and_socket(self):
        host = self.host(start=False)
        host.config.write_bytes(b'invalid')
        run = subprocess.run([str(HOST), '--headless-test'], env=host.env, capture_output=True, timeout=3)
        self.assertNotEqual(run.returncode, 0)
        self.assertFalse((host.profile / 'control.sock').exists())
        host.config.write_text(json.dumps(DEFAULT))
        host.start()

    def test_quit_acknowledges_then_exits_normally_without_kill_fallback(self):
        host = self.host()
        self.assertEqual(host.rpc({'op': 'quit'}), {'ok': True, 'result': {'state': 'stopping'}})
        out, err = host.process.communicate(timeout=3)
        self.assertEqual(host.process.returncode, 0, err.decode())
        self.assertEqual(json.loads(out), {'ok': True, 'result': {'state': 'stopped'}})
        self.assertFalse((host.profile / 'control.sock').exists())


if __name__ == '__main__':
    unittest.main(verbosity=2)
