"""Integration against the real native daemon in an isolated headless profile."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest

REPO = Path(__file__).resolve().parents[1]
BINARY = REPO / '.build/swift/debug/RuycastApp'
CLI = REPO / '.build/rust/release/ruycast'


class CLIIntegration(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        base = Path(os.environ.get('VIGGO_TEST_ROOT', REPO / '.build/test-profiles'))
        base.mkdir(parents=True, exist_ok=True)
        cls.tmp = tempfile.TemporaryDirectory(dir=base)
        cls.root = Path(cls.tmp.name)
        cls.env = dict(os.environ, RUYCAST_HOME=str(cls.root), RUYCAST_APP_BINARY=str(BINARY))
        subprocess.run([str(BINARY), '--init-config'], env=cls.env, check=True, capture_output=True)
        cls.config = json.loads((cls.root / 'config.json').read_text())
        cls.config['commands'].extend([
            {'id': 'test.echo', 'title': 'Test echo', 'kind': 'script', 'target': '/usr/bin/printf', 'arguments': ['%s', 'literal $(not a command)\nΩ'], 'aliases': ['echo-test']},
            {'id': 'test.fail', 'title': 'Test failure', 'kind': 'script', 'target': '/usr/bin/false'},
            {'id': 'test.timeout', 'title': 'Test timeout', 'kind': 'script', 'target': '/bin/sleep', 'arguments': ['3'], 'timeout': 0.1},
            {'id': 'test.output', 'title': 'Test large output', 'kind': 'script', 'target': '/usr/bin/python3', 'arguments': ['-c', 'import sys;sys.stdout.write("x"*200000)']},
        ])
        (cls.root / 'config.json').write_text(json.dumps(cls.config))
        cls.proc = subprocess.Popen([str(BINARY), '--headless'], env=cls.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(100):
            if (cls.root / 'control.sock').exists():
                try:
                    if cls.cli('status')['ok']:
                        break
                except Exception:
                    pass
            if cls.proc.poll() is not None:
                raise RuntimeError('Headless native daemon exited')
            time.sleep(.05)
        else:
            raise RuntimeError('Headless daemon did not become ready')

    @classmethod
    def cli(cls, *args, ok=None):
        p = subprocess.run([str(CLI), *args], env=cls.env, capture_output=True, text=True, timeout=10)
        r = json.loads(p.stdout)
        if p.returncode != (0 if r.get('ok') else 1):
            raise AssertionError('Exit status disagrees with JSON: ' + p.stdout + p.stderr)
        if ok is not None and r['ok'] != ok:
            raise AssertionError(r)
        return r

    def test_clipboard_import_keeps_file_metadata_and_replays_without_duplicates(self):
        date=__import__('datetime').datetime.now(__import__('datetime').timezone.utc).isoformat()
        items=[{'id':'raycast-fixture-text','text':'test imported text','date':date,'source':'fixture'}, {'id':'raycast-fixture-file','text':'Files: fixture.txt','date':date,'source':'fixture','contentType':'files','fileURLs':['file:///nonexistent/fixture.txt']}]
        def request(items):
            with socket.socket(socket.AF_UNIX,socket.SOCK_STREAM) as sock:
                sock.connect(str(self.root/'control.sock'));sock.sendall(json.dumps({'op':'clipboard.import','items':items}).encode()+b'\n')
                with sock.makefile() as stream:return json.loads(stream.readline())
        self.assertTrue(request(items)['ok'])
        self.assertEqual(request(items)['result']['deduplicated'],2)
        files=next(c for c in self.cli('clipboard','list',ok=True)['result'] if c['id']=='raycast-fixture-file')
        self.assertEqual(files['fileURLs'],items[1]['fileURLs'])
        self.assertFalse(request([dict(items[0],id='unsafe',dataFile='../private')])['ok'])
        self.cli('clipboard','clear',ok=True)

    def test_appearance_preferences_validate_and_persist(self):
        for theme in ['system', 'light', 'dark', 'graphite']:
            self.cli('config', 'set', 'theme', json.dumps(theme), ok=True)
            self.assertEqual(self.cli('config', 'get', ok=True)['result']['theme'], theme)
        before = self.cli('config', 'get', ok=True)['sha256']
        self.cli('config', 'set', 'theme', '"unknown"', ok=False)
        self.assertEqual(self.cli('config', 'get', ok=True)['sha256'], before)
        for visible in [False, True]:
            self.cli('config', 'set', 'menuBarVisible', json.dumps(visible), ok=True)
            self.assertEqual(self.cli('config', 'get', ok=True)['result']['menuBarVisible'], visible)
        self.cli('config', 'set', 'menuBarVisible', '"yes"', ok=False)

    def test_launcher_management_commands_and_settings_hotkey(self):
        for alias, identifier in [('open-config', 'builtin.open-config'), ('reload-config', 'builtin.reload-config'), ('settings', 'builtin.settings')]:
            self.assertIn(identifier, [r['id'] for r in self.cli('search', alias, ok=True)['result']])
        snapshot = self.cli('config', 'get', ok=True)['result']
        settings = {'id':'builtin.settings','title':'Ruycast Settings','kind':'builtin','target':'settings','hotkey':'ctrl+alt+cmd+s'}
        commands = [c for c in snapshot['commands'] if c['id'] != settings['id']] + [settings]
        self.cli('config', 'set', 'commands', json.dumps(commands), ok=True)
        self.assertEqual(next(c for c in self.cli('config','get',ok=True)['result']['commands'] if c['id']==settings['id'])['hotkey'],settings['hotkey'])
        self.assertEqual(sum(r['id']==settings['id'] for r in self.cli('search','settings',ok=True)['result']),1)

    @classmethod
    def tearDownClass(cls):
        try:
            cls.cli('quit')
            cls.proc.wait(timeout=5)
        finally:
            if cls.proc.poll() is None:
                cls.proc.kill()
            cls.tmp.cleanup()

    def tearDown(self):
        (self.root / 'config.json').write_text(json.dumps(self.config))
        self.cli('reload', ok=True)

    def test_existing_quick_note_provider_in_fixture_vault(self):
        provider=Path(os.environ.get('RUYCAST_CAPTURE_PROVIDER',''))
        if not provider.is_file():
            self.skipTest('Optional capture provider not configured')
        vault=self.root/'fixture-vault';inbox=vault/'00-inbox';inbox.mkdir(parents=True,exist_ok=True)
        command={'id':'capture.quick-note','title':'Quick Note','kind':'script','target':'/bin/sh','arguments':[str(provider),str(inbox),'{input}',str(vault)],'inputPrompt':'Note'}
        self.cli('config','add',json.dumps(command),ok=True)
        text='Rustcast provider fixture\n- [ ] literal text stays intact'
        response=self.cli('run','capture.quick-note','--input',json.dumps({'input':text}),ok=True)
        saved=Path(response['result']['note_path'])
        self.assertTrue(saved.is_relative_to(vault))
        self.assertIn(text,saved.read_text())
        self.assertEqual(response['result'].get('kind'),'note')

    def test_script_timeout_and_bounded_output(self):
        start = time.monotonic()
        self.assertIn('timed out', self.cli('run', 'test.timeout', ok=False)['error'])
        self.assertLess(time.monotonic() - start, 2)
        r = self.cli('run', 'test.output', ok=True)
        self.assertEqual(len(r['result']['stdout']), 65536)

    def test_clipboard_control_private_permissions_and_no_ui(self):
        self.cli('clipboard', 'pause', ok=True)
        self.assertTrue(self.cli('status')['result']['clipboardPaused'])
        self.cli('clipboard', 'resume', ok=True)
        self.cli('clipboard', 'clear', ok=True)
        self.assertEqual(self.cli('clipboard', 'list')['result'], [])
        self.cli('clipboard', 'copy', 'missing', ok=False)
        self.cli('show', ok=False)
        self.assertEqual((self.root.stat().st_mode & 0o777), 0o700)
        self.assertEqual(((self.root / 'control.sock').stat().st_mode & 0o777), 0o600)
        self.assertEqual(((self.root / 'clipboard.json').stat().st_mode & 0o777), 0o600)

    def test_second_instance_does_not_replace_socket(self):
        before = self.cli('status')['result']['pid']
        second = subprocess.run([str(BINARY), '--headless'], env=self.env, capture_output=True, timeout=5)
        self.assertIn(b'already running', second.stderr)
        self.assertEqual(self.cli('status')['result']['pid'], before)

    def test_script_timeout_kills_descendants_and_inherited_pipes(self):
        sentinel = str(self.root / 'orphan-sentinel')
        grandchild = "import time;from pathlib import Path;time.sleep(1);Path(%r).write_text('bad')" % sentinel
        code = "import subprocess,sys,time;subprocess.Popen([sys.executable,'-c',%r]);time.sleep(10)" % grandchild
        command = dict(id='test.tree', title='Tree', kind='script', target='/usr/bin/python3', arguments=['-c', code], timeout=0.15)
        self.cli('config', 'add', json.dumps(command), ok=True)
        start = time.monotonic()
        self.cli('run', 'test.tree', ok=False)
        self.assertLess(time.monotonic() - start, 2)
        time.sleep(1)
        self.assertFalse(Path(sentinel).exists())
        command.update(id='test.pipes', arguments=['-c', "import subprocess;subprocess.Popen(['/bin/sleep','10'])"], timeout=2)
        self.cli('config', 'add', json.dumps(command), ok=True)
        start = time.monotonic()
        self.cli('run', 'test.pipes', ok=True)
        self.assertLess(time.monotonic() - start, 2)
