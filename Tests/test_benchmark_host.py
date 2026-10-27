"""The host measurement driver must distinguish real success from forced cleanup."""
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest


DRIVER = Path(__file__).resolve().parents[1] / "scripts/benchmark-host.py"
HOST = r'''#!/usr/bin/env python3
import json, os, signal, socket, sys, time
from pathlib import Path
root = Path(os.environ["RUYCAST_HOME"])
mode = os.environ["F1_DRIVER_TEST_MODE"]
events = Path(os.environ["F1_DRIVER_TEST_EVENTS"])
def event(value):
    with events.open("a") as stream:
        stream.write(json.dumps({"pid": os.getpid(), "event": value}) + "\n")
if "--init-config" in sys.argv:
    if mode == "init_hang":
        time.sleep(30)
    (root / "config.json").write_text("{}")
    sys.exit(0)
def terminated(*_):
    event("terminate")
    if mode != "quit_ignores_terminate":
        sys.exit(0)
signal.signal(signal.SIGTERM, terminated)
(root / "control.sock").unlink(missing_ok=True)
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(str(root / "control.sock"))
server.listen()
last_request = None
event("started")
while True:
    connection, _ = server.accept()
    with connection:
        payload = b""
        while b"\n" not in payload:
            chunk = connection.recv(65536)
            if not chunk:
                break
            payload += chunk
        request = json.loads(payload)
        now = time.monotonic()
        if mode == "clipboard_hang" and request["op"] == "clipboard.list":
            time.sleep(30)
        if mode in ("idle_hang", "idle_drip") and request["op"] == "status" and last_request is not None and now-last_request > .12:
            event("idle-status-hang")
            if mode == "idle_drip":
                while True:
                    connection.sendall(b" ")
                    time.sleep(.05)
            time.sleep(30)
        last_request = now
        if request["op"] == "status":
            result = {"headless": True, "configPath": str(root/"config.json"), "commands": 500}
        elif request["op"] == "search":
            result = [{"id": "fixture.42"}]
        else:
            result = []
        connection.sendall(json.dumps({"ok": True, "result": result}).encode()+b"\n")
        if request["op"] == "quit":
            event("quit")
            if mode not in ("quit_stays_alive", "quit_ignores_terminate"):
                break
'''
CLI = r'''#!/usr/bin/env python3
import os, time
if os.environ["F1_DRIVER_TEST_MODE"] == "cli_hang":
    time.sleep(30)
print('{"ok":true}')
'''
PROBE = r'''#!/usr/bin/env python3
import json, os, time
if os.environ["F1_DRIVER_TEST_MODE"] == "probe_hang":
    time.sleep(30)
print(json.dumps({"cpuNS":0,"packageIdleWakeups":0,"interruptWakeups":0,
 "footprintBytes":1048576,"diskReadBytes":0,"diskWriteBytes":0,"logicalWriteBytes":0}))
'''


class BenchmarkHostTests(unittest.TestCase):
    def run_driver(self, mode):
        with tempfile.TemporaryDirectory(prefix="ruycast-driver-test-") as directory:
            root = Path(directory)
            for name, body in [("host", HOST), ("cli", CLI), ("probe", PROBE)]:
                path = root / name
                path.write_text(body)
                path.chmod(0o700)
            output = root / "result.json"
            events = root / "events.jsonl"
            env = dict(os.environ, F1_DRIVER_TEST_MODE=mode, F1_DRIVER_TEST_EVENTS=str(events))
            args = ["python3", str(DRIVER), "--host", str(root/"host"),
                    "--cli", str(root/"cli"), "--probe", str(root/"probe"),
                    "--out", str(output), "--cold-starts", "1", "--samples", "1",
                    "--active-seconds", ".02", "--idle-seconds", ".2",
                    # Distinguish deliberate 30s hangs without imposing a 500ms
                    # process-start performance budget on a busy development Mac.
                    "--operation-timeout", "2"]
            process = subprocess.Popen(args, env=env, stdout=subprocess.PIPE,
                                       stderr=subprocess.PIPE, text=True, start_new_session=True)
            try:
                stdout, stderr = process.communicate(timeout=12)
                self.assertTrue(output.is_file(), stdout + stderr)
                result = json.loads(output.read_text())
                recorded = [json.loads(line) for line in events.read_text().splitlines()] if events.exists() else []
                return process.returncode, result, recorded
            finally:
                # Every fixture child belongs to this exclusively created process group.
                # This also cleans up the old, deliberately unbounded red implementation.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                if process.poll() is None:
                    process.wait(timeout=5)

    def test_success_requires_idle_responsiveness_and_graceful_exit(self):
        code, result, events = self.run_driver("ok")
        self.assertEqual(code, 0, result)
        self.assertTrue(result["ok"])
        self.assertTrue(result["idleEndpoint"]["statusVerified"])
        self.assertTrue(result["idleEndpoint"]["searchVerified"])
        self.assertTrue(result["finalShutdown"]["graceful"])
        self.assertFalse(result["finalShutdown"]["forcedCleanup"])
        self.assertEqual(sum(row["event"] == "quit" for row in events), 2)
        self.assertFalse(any(row["event"] == "terminate" for row in events))

    def test_forced_termination_is_a_failure_even_when_child_exits_zero(self):
        code, result, events = self.run_driver("quit_stays_alive")
        self.assertNotEqual(code, 0)
        self.assertFalse(result["ok"])
        self.assertTrue(result["cleanup"]["forcedCleanup"])
        self.assertEqual(result["cleanup"]["exitCode"], 0)
        self.assertTrue(any(row["event"] == "terminate" for row in events))

    def test_ignored_termination_is_killed_and_never_marked_successful(self):
        code, result, _ = self.run_driver("quit_ignores_terminate")
        self.assertNotEqual(code, 0)
        self.assertFalse(result["ok"])
        self.assertEqual(result["cleanup"]["method"], "kill")
        self.assertEqual(result["cleanup"]["exitCode"], -signal.SIGKILL)

    def test_child_timeouts_write_failure_receipts(self):
        for mode in ["init_hang", "cli_hang", "probe_hang"]:
            with self.subTest(mode=mode):
                code, result, _ = self.run_driver(mode)
                self.assertNotEqual(code, 0)
                self.assertFalse(result["ok"])
                self.assertEqual(result["errorType"], "TimeoutExpired")
                if mode == "probe_hang":
                    self.assertFalse(result["active"]["complete"])

    def test_idle_socket_hang_cannot_pass_on_healthy_os_counters(self):
        code, result, events = self.run_driver("idle_hang")
        self.assertNotEqual(code, 0)
        self.assertFalse(result["ok"])
        self.assertEqual(result["phase"], "idle-endpoint")
        self.assertTrue(any(row["event"] == "idle-status-hang" for row in events))
        self.assertTrue(result["idle"]["complete"])
        self.assertGreaterEqual(len(result["idle"]["samples"]), 2)

    def test_partial_response_cannot_extend_rpc_deadline_indefinitely(self):
        code, result, events = self.run_driver("idle_drip")
        self.assertNotEqual(code, 0)
        self.assertFalse(result["ok"])
        self.assertEqual(result["phase"], "idle-endpoint")
        self.assertEqual(result["errorType"], "TimeoutError")
        self.assertTrue(any(row["event"] == "idle-status-hang" for row in events))

    def test_failure_keeps_completed_requests_and_partial_samples(self):
        code, result, _ = self.run_driver("clipboard_hang")
        self.assertNotEqual(code, 0)
        self.assertFalse(result["ok"])
        self.assertEqual(result["phase"], "active")
        self.assertFalse(result["active"]["complete"])
        self.assertGreaterEqual(len(result["active"]["samples"]), 1)
        self.assertEqual(result["active"]["requests"], 1)


if __name__ == "__main__":
    unittest.main()
