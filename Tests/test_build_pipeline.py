"""Real subprocess/Unix-lock checks; expensive compilers are disposable stubs."""
import fcntl
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]


class BuildPipelineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=ROOT / '.build')
        self.root = Path(self.tmp.name)
        scripts = self.root / 'scripts'
        scripts.mkdir()
        for name in ['build_receipt.py', 'prepare_release.py', 'build-native.sh',
                     'check.sh', 'check-locked.sh', 'check-performance.sh',
                     'check-performance-locked.sh', 'check-settings.sh', 'check-settings-locked.sh']:
            if (ROOT / 'scripts' / name).exists():
                shutil.copy2(ROOT / 'scripts' / name, scripts / name)
        (scripts / 'build-native-locked.sh').write_text('''#!/bin/bash
set -eu
if [[ -e .build/fail-build ]]; then exit 7; fi
if [[ " $* " == *" -c release "* ]]; then
  touch .build/competing-build-entered
  rm -f .build/swift/debug/RuycastApp
  exit 0
fi
mkdir -p .build/swift/debug/Modules .build/swift/debug/CRuycastCore.build .build/swift/release .build/rust/release
echo module > .build/swift/debug/Modules/LauncherModels.swiftmodule
echo modulemap > .build/swift/debug/CRuycastCore.build/module.modulemap
for file in .build/swift/debug/RuycastApp .build/swift/release/RuycastApp .build/swift/debug/LauncherCoreChecks .build/rust/release/ruycast; do
  printf '#!/bin/bash\\nexit 0\\n' > "$file"
  chmod +x "$file"
done
''')
        (self.root / 'bin').mkdir()
        cargo = self.root / 'bin' / 'cargo'
        cargo.write_text('#!/bin/bash\nexit 0\n')
        cargo.chmod(0o755)
        self.env = dict(os.environ, PATH=str(self.root / 'bin') + os.pathsep + os.environ['PATH'])
        tests = self.root / 'Tests'
        tests.mkdir()
        (tests / 'test_hold.py').write_text('''import time, unittest
from pathlib import Path
class Hold(unittest.TestCase):
 def test_artifact_survives(self):
  root=Path.cwd();(root/'.build/checks-started').touch()
  end=time.monotonic()+10
  while not (root/'.build/release-checks').exists():
   if time.monotonic()>end:raise RuntimeError('Fixture release timed out')
   time.sleep(.005)
  self.assertFalse((root/'.build/fail-checks').exists(), 'Injected failed check')
  self.assertTrue((root/'.build/swift/debug/RuycastApp').is_file(), 'Concurrent build removed checked executable')
''')
        # Both other public pipelines use the same production lock boundary.
        # Their native behavior has separate integration/performance fixtures.
        for name in ['check-performance-locked.sh', 'check-settings-locked.sh']:
            if (scripts / name).exists():
                (scripts / name).write_text('cd "$(dirname "$0")/.."\npython3 -m unittest discover -s Tests -p "test_*.py"\n')
        self.processes = []

    def tearDown(self):
        (self.root / '.build').mkdir(exist_ok=True)
        (self.root / '.build/release-checks').touch()
        for proc in self.processes:
            try:
                proc.communicate(timeout=6)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.communicate(timeout=3)
        self.tmp.cleanup()

    def start(self, command):
        proc = subprocess.Popen(command, cwd=self.root, env=self.env,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.processes.append(proc)
        return proc

    def wait_file(self, name, proc):
        path = self.root / '.build' / name
        end = time.monotonic() + 6
        while not path.exists():
            if proc.poll() is not None:
                out, err = proc.communicate()
                self.fail(f'Fixture exited early ({proc.returncode}): {out} {err}')
            if time.monotonic() > end:
                self.fail('Fixture did not reach ' + name)
            time.sleep(.005)

    def assert_lease_held(self):
        with (self.root / '.build/artifacts.lock').open('a') as guard:
            with self.assertRaises(BlockingIOError):
                fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def competing_build(self):
        code = "from pathlib import Path;import os,sys;Path('.build/build-requested').touch();os.execv(sys.executable,[sys.executable,'scripts/build_receipt.py','build','-c','release'])"
        proc = self.start([sys.executable, '-c', code])
        self.wait_file('build-requested', proc)
        return proc

    def release(self, pipeline, build):
        (self.root / '.build/release-checks').touch()
        out, err = pipeline.communicate(timeout=8)
        self.assertEqual(pipeline.returncode, 0, out + err)
        out, err = build.communicate(timeout=8)
        self.assertEqual(build.returncode, 0, out + err)
        self.assertTrue((self.root / '.build/competing-build-entered').exists())

    def test_checks_keep_debug_artifacts_until_the_last_consumer(self):
        check = self.start(['bash', 'scripts/check.sh'])
        self.wait_file('checks-started', check)
        build = self.competing_build()
        self.assert_lease_held()
        self.assertFalse((self.root / '.build/competing-build-entered').exists())
        self.release(check, build)

    def test_lease_survives_controller_crash_while_consumer_is_live(self):
        check = self.start(['bash', 'scripts/check.sh'])
        self.wait_file('checks-started', check)
        check.kill()
        check.wait(timeout=3)
        self.assert_lease_held()
        build = self.competing_build()
        self.assertFalse((self.root / '.build/competing-build-entered').exists())
        (self.root / '.build/release-checks').touch()
        out, err = build.communicate(timeout=8)
        self.assertEqual(build.returncode, 0, out + err)
        self.assertTrue((self.root / '.build/competing-build-entered').exists())

    def test_lease_survives_controller_interrupt_while_consumer_is_live(self):
        check = self.start(['bash', 'scripts/check.sh'])
        self.wait_file('checks-started', check)
        check.send_signal(signal.SIGINT)
        check.wait(timeout=3)
        self.assert_lease_held()
        build = self.competing_build()
        self.assertFalse((self.root / '.build/competing-build-entered').exists())
        (self.root / '.build/release-checks').touch()
        out, err = build.communicate(timeout=8)
        self.assertEqual(build.returncode, 0, out + err)

    def test_source_edit_during_checks_cannot_receive_passed_receipt(self):
        check = self.start(['bash', 'scripts/check.sh'])
        self.wait_file('checks-started', check)
        (self.root / 'Tests/test_hold.py').write_text((self.root / 'Tests/test_hold.py').read_text() + '\n# concurrent edit\n')
        (self.root / '.build/release-checks').touch()
        out, err = check.communicate(timeout=8)
        self.assertNotEqual(check.returncode, 0, out + err)
        self.assertIn('changed during verification', err)
        self.assertFalse((self.root / '.build/receipts/check.json').exists())

    def test_artifact_edit_during_checks_cannot_receive_passed_receipt(self):
        check = self.start(['bash', 'scripts/check.sh'])
        self.wait_file('checks-started', check)
        binary = self.root / '.build/rust/release/ruycast'
        binary.write_bytes(binary.read_bytes() + b'\n# changed\n')
        (self.root / '.build/release-checks').touch()
        out, err = check.communicate(timeout=8)
        self.assertNotEqual(check.returncode, 0, out + err)
        self.assertIn('changed during verification', err)
        self.assertFalse((self.root / '.build/receipts/check.json').exists())

    def test_settings_and_performance_share_the_whole_pipeline_lease(self):
        # Build stub writes a debug host for the disposable consumer in both cases.
        script = self.root / 'scripts/build-native-locked.sh'
        script.write_text(script.read_text().replace('if [[ " $* " == *" -c release "* ]]; then', 'if [[ " $* " == *" -c release "* && -e .build/checks-started ]]; then'))
        for mode in ['settings', 'performance']:
            with self.subTest(mode=mode):
                for name in ['checks-started', 'release-checks', 'competing-build-entered', 'build-requested']:
                    (self.root / '.build' / name).unlink(missing_ok=True)
                check = self.start(['bash', f'scripts/check-{mode}.sh'])
                self.wait_file('checks-started', check)
                build = self.competing_build()
                self.assert_lease_held()
                self.assertFalse((self.root / '.build/competing-build-entered').exists())
                self.release(check, build)

    def test_failed_build_releases_lease_and_does_not_start_consumer(self):
        (self.root / '.build').mkdir()
        (self.root / '.build/fail-build').touch()
        check = self.start(['bash', 'scripts/check.sh'])
        out, err = check.communicate(timeout=6)
        self.assertEqual(check.returncode, 7, out + err)
        self.assertFalse((self.root / '.build/checks-started').exists())
        self.assertFalse((self.root / '.build/receipts/check.json').exists())
        (self.root / '.build/fail-build').unlink()
        build = self.competing_build()
        out, err = build.communicate(timeout=6)
        self.assertEqual(build.returncode, 0, out + err)

    def test_receipt_revalidation_and_failed_attempt_remove_old_pass(self):
        check = self.start(['bash', 'scripts/check.sh'])
        self.wait_file('checks-started', check)
        (self.root / '.build/release-checks').touch()
        out, err = check.communicate(timeout=8)
        self.assertEqual(check.returncode, 0, out + err)
        receipt = self.root / '.build/receipts/check.json'
        value = json.loads(receipt.read_text())
        self.assertEqual(value['status'], 'passed')
        self.assertEqual([r['stage'] for r in value['steps']], ['rustTests', 'build', 'verification'])
        verify = self.start([sys.executable, 'scripts/build_receipt.py', 'verify-check'])
        out, err = verify.communicate(timeout=5)
        self.assertEqual(verify.returncode, 0, out + err)
        source = self.root / 'Tests/test_hold.py'
        original = source.read_text()
        source.write_text(original + '\n# later edit\n')
        verify = self.start([sys.executable, 'scripts/build_receipt.py', 'verify-check'])
        out, err = verify.communicate(timeout=5)
        self.assertNotEqual(verify.returncode, 0, out + err)
        self.assertIn('stale', err)
        source.write_text(original)
        docs = self.root / 'docs'
        docs.mkdir()
        vision = docs / 'vision.json'
        vision.write_text('{}')
        verify = self.start([sys.executable, 'scripts/build_receipt.py', 'verify-check'])
        out, err = verify.communicate(timeout=5)
        self.assertNotEqual(verify.returncode, 0, out + err)
        self.assertIn('stale', err)
        vision.unlink()
        module = self.root / '.build/swift/debug/Modules/LauncherModels.swiftmodule'
        original_module = module.read_bytes()
        module.write_bytes(b'changed module')
        verify = self.start([sys.executable, 'scripts/build_receipt.py', 'verify-check'])
        out, err = verify.communicate(timeout=5)
        self.assertNotEqual(verify.returncode, 0, out + err)
        self.assertIn('stale', err)
        module.write_bytes(original_module)
        binary = self.root / '.build/rust/release/ruycast'
        binary.write_bytes(binary.read_bytes() + b'\n# later artifact edit\n')
        verify = self.start([sys.executable, 'scripts/build_receipt.py', 'verify-check'])
        out, err = verify.communicate(timeout=5)
        self.assertNotEqual(verify.returncode, 0, out + err)
        self.assertIn('stale', err)
        # A subsequent build restores binaries but its failed consumer must not
        # leave the first run's passed receipt behind.
        (self.root / '.build/fail-checks').touch()
        check = self.start(['bash', 'scripts/check.sh'])
        out, err = check.communicate(timeout=8)
        self.assertNotEqual(check.returncode, 0, out + err)
        self.assertFalse(receipt.exists())
