"""Exercise real host startup/shutdown in owned, short-path profiles only."""
import json
import os
from pathlib import Path
import socket
import stat
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
HOST = Path(os.environ.get('RUYCAST_TEST_HOST', ROOT / '.build/swift/debug/RuycastApp')).resolve()


class SocketRecoveryTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='sr-', dir=ROOT / '.build')
        self.root = Path(self.tmp.name)
        self.path = self.root / 'control.sock'
        self.env = {k: v for k, v in os.environ.items() if k not in
                    ['RUYCAST_HOME', 'RUSTCAST_HOME', 'RUYCAST_CONFIG_DIR', 'RUYCAST_APP_BINARY']}
        self.env['RUYCAST_HOME'] = str(self.root)
        self.processes = []

    def tearDown(self):
        for process in self.processes:
            if process.poll() is None:
                process.terminate()
            try:
                process.communicate(timeout=4)
            except subprocess.TimeoutExpired:
                process.kill()
                process.communicate(timeout=3)
        self.tmp.cleanup()

    def launch(self):
        process = subprocess.Popen([str(HOST), '--headless'], cwd=ROOT, env=self.env,
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.processes.append(process)
        return process

    def connect(self):
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(3)
        try:
            client.connect(str(self.path))
        except BaseException:
            client.close()
            raise
        return client

    def request(self, client, value, allow_starting=False):
        client.sendall(json.dumps(value).encode() + b'\n')
        data = b''
        while not data.endswith(b'\n'):
            chunk = client.recv(65536)
            self.assertTrue(chunk, 'Host closed before replying')
            data += chunk
        response = json.loads(data)
        if allow_starting and not response['ok'] and response.get('error') == 'Starting up; retry.':
            return None
        self.assertTrue(response['ok'], response)
        return response['result']

    def start(self):
        process = self.launch()
        end = time.monotonic() + 6
        while time.monotonic() < end:
            if process.poll() is not None:
                out, err = process.communicate()
                self.fail('Owned fixture host exited early: ' + out + err)
            try:
                with self.connect() as client:
                    response = self.request(client, {'op': 'status'}, allow_starting=True)
                if response is None:
                    time.sleep(.01)
                    continue
                self.assertEqual(response['pid'], process.pid)
                self.assertTrue(response['headless'])
                self.assertEqual(Path(response['profile']).resolve(), self.root.resolve())
                return process
            except (OSError, ValueError):
                time.sleep(.01)
        self.fail('Owned fixture host did not become ready')

    def rejected_start(self):
        process = self.launch()
        try:
            out, err = process.communicate(timeout=4)
        except subprocess.TimeoutExpired:
            self.fail('Startup unexpectedly accepted or blocked on an occupied path')
        self.assertNotEqual(process.returncode, 0, out + err)
        return out + err

    def quit(self, process, client=None):
        if client is None:
            with self.connect() as connection:
                self.request(connection, {'op': 'quit'})
        else:
            self.request(client, {'op': 'quit'})
        out, err = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0, out + err)

    def test_regular_endpoint_is_preserved_then_retry_recovers(self):
        payload = b'An unrelated file must survive failed server startup.'
        self.path.write_bytes(payload)
        self.rejected_start()
        self.assertEqual(self.path.read_bytes(), payload)
        self.path.unlink()
        self.quit(self.start())
        self.assertFalse(self.path.exists())

    def test_linked_endpoint_and_target_are_preserved(self):
        target = self.root / 'endpoint-target'
        target.write_bytes(b'preserved target')
        self.path.symlink_to(target)
        self.rejected_start()
        self.assertTrue(self.path.is_symlink())
        self.assertEqual(target.read_bytes(), b'preserved target')
        self.path.unlink()
        self.quit(self.start())

    def test_live_endpoint_without_instance_lock_is_preserved(self):
        with socket.socket(socket.AF_UNIX) as foreign:
            foreign.bind(str(self.path))
            foreign.listen(4)
            inode = self.path.stat().st_ino
            self.rejected_start()
            self.assertEqual(self.path.stat().st_ino, inode)
        # The same path is now a stale socket; startup must recover normally.
        self.quit(self.start())

    def test_stale_endpoint_recovers_and_graceful_quit_removes_it(self):
        with socket.socket(socket.AF_UNIX) as stale:
            stale.bind(str(self.path))
        self.quit(self.start())
        self.assertFalse(self.path.exists())

    def test_graceful_stop_preserves_replacement_regular_file(self):
        process = self.start()
        with self.connect() as client:
            self.path.unlink()
            self.path.write_bytes(b'replacement while host is alive')
            self.quit(process, client)
        self.assertEqual(self.path.read_bytes(), b'replacement while host is alive')

    def test_graceful_stop_preserves_replacement_socket(self):
        process = self.start()
        with self.connect() as client, socket.socket(socket.AF_UNIX) as replacement:
            self.path.unlink()
            replacement.bind(str(self.path))
            replacement.listen(4)
            inode = self.path.stat().st_ino
            self.quit(process, client)
            self.assertEqual(self.path.stat().st_ino, inode)
            with self.connect():
                pass
        self.quit(self.start())

    def test_linked_instance_lock_is_preserved_then_retry_recovers(self):
        target = self.root / 'lock-target'
        target.write_bytes(b'preserved lock target')
        link = self.root / 'instance.lock'
        link.symlink_to(target)
        self.rejected_start()
        self.assertTrue(link.is_symlink())
        self.assertEqual(target.read_bytes(), b'preserved lock target')
        link.unlink()
        self.quit(self.start())

    def test_fifo_instance_lock_is_rejected_without_blocking(self):
        lock = self.root / 'instance.lock'
        os.mkfifo(lock, 0o600)
        self.rejected_start()
        self.assertTrue(stat.S_ISFIFO(lock.stat().st_mode))
        lock.unlink()
        self.quit(self.start())
