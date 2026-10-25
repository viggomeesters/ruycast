"""Failed activation rolls back only our unchanged candidate bytes."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import unittest

REPO = Path(__file__).resolve().parents[1]
CLI = REPO / '.build/rust/release/ruycast'

class ConfigActivationTests(unittest.TestCase):
    def activation(self, external=False):
        with tempfile.TemporaryDirectory(prefix='rc-activation-', dir=os.environ.get('TMPDIR')) as folder:
            root = Path(folder)
            config = root / 'config.json'
            original = (REPO / 'config/default.json').read_bytes()
            config.write_bytes(original)
            newer = original + b'\n\n'
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as server:
                server.bind(str(root / 'control.sock'));server.listen(2);server.settimeout(5)
                failures = []
                def respond():
                    try:
                        with server.accept()[0] as connection:
                            request = json.loads(connection.makefile('rb').readline())
                            self.assertEqual(request['op'], 'reload')
                            if external:
                                config.write_bytes(newer)
                            connection.sendall(b'{"ok":false,"error":"Hotkey unavailable"}\n')
                        if not external:
                            with server.accept()[0] as connection:
                                connection.makefile('rb').readline()
                                self.assertEqual(config.read_bytes(), original)
                                connection.sendall(b'{"ok":true}\n')
                    except Exception as error:
                        failures.append(error)
                worker = threading.Thread(target=respond);worker.start()
                result = subprocess.run([str(CLI), 'config', 'set', 'clipboard.limit', '51'], env=dict(os.environ, RUYCAST_HOME=folder), capture_output=True, text=True, timeout=8)
                worker.join(6)
                self.assertFalse(worker.is_alive())
                self.assertFalse(failures, str(failures))
                response = json.loads(result.stdout)
                self.assertFalse(response['ok'])
                self.assertEqual(response['rolledBack'], not external)
                self.assertEqual(response['runtimeRecovered'], not external)
                self.assertEqual(config.read_bytes(), newer if external else original)
    def test_rejected_candidate_restores_previous_bytes_and_runtime(self):
        self.activation()
    def test_newer_external_edit_is_never_rolled_back(self):
        self.activation(external=True)
