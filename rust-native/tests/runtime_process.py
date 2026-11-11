"""Visible shared Host/fixture data for remaining Python consumers until #360.
The fifteen original runtime methods now execute in runtime_process.rs.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
HOST = Path(os.environ.get('RUYCAST_TEST_APP_HOST', ROOT / 'target/headless-tools/debug/ruycast-native'))
CLI = Path(os.environ.get('RUYCAST_TEST_CLI', ROOT.parent / '.build/rust/release/ruycast'))
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
