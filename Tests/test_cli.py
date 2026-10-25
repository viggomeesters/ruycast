"""Integration against the real native daemon in an isolated headless profile."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import unittest
from concurrent.futures import ThreadPoolExecutor

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

    def test_atomic_replacement_preserves_new_fields_and_rejects_drift(self):
        snapshot=self.cli('config','get',ok=True)
        candidate=dict(snapshot['result'],hyperKeyEnabled=True)
        candidate['commands']=candidate['commands']+[{'id':'chosen-app','title':'Chosen app','kind':'url','target':'https://example.com','applicationID':'com.apple.Safari','hotkey':'ctrl+alt+delete'}]
        path=self.root/'candidate.json';path.write_text(json.dumps(candidate))
        self.cli('config','replace','@'+str(path),ok=False)
        self.cli('config','replace','@'+str(path),'--expect-sha256','0'*64,ok=False)
        self.assertEqual(self.cli('config','get',ok=True)['sha256'],snapshot['sha256'])
        self.cli('config','replace','@'+str(path),'--expect-sha256',snapshot['sha256'],ok=True)
        readback=self.cli('config','get',ok=True)['result']
        self.assertTrue(readback['hyperKeyEnabled'])
        self.assertEqual(readback['commands'][-1]['applicationID'],'com.apple.Safari')
        self.cli('config','replace','@'+str(path),'--expect-sha256',snapshot['sha256'],ok=False)

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

    def test_settings_command_is_searchable_and_headless_guarded(self):
        rows = self.cli('search', 'settings', ok=True)['result']
        self.assertIn('builtin.settings', [row['id'] for row in rows])
        result = self.cli('run', 'builtin.settings', ok=False)
        self.assertIn('headless', result['error'])

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

    def test_snippets_and_quicklinks_and_input_are_literal(self):
        snippet={'id':'snippet.test','title':'Greeting','kind':'snippet','target':'Hello {cursor}','keyword':';hello'}
        self.cli('snippets','add',json.dumps(snippet),ok=True)
        rows=self.cli('snippets','list',ok=True)['result']
        self.assertEqual([r['id'] for r in rows],['snippet.test'])
        self.assertEqual(self.cli('search',';hello',ok=True)['result'][0]['id'],'snippet.test')
        link={'id':'link.test','title':'Find','kind':'url','target':'https://example.com/?q={query}','aliases':['find-test']}
        self.cli('quicklinks','add',json.dumps(link),ok=True)
        self.assertEqual(self.cli('quicklinks','list','find-test',ok=True)['result'][0]['target'],link['target'])
        script={'id':'test.input','title':'Input','kind':'script','target':'/usr/bin/printf','arguments':['%s','{input}'],'inputPrompt':'Text'}
        self.cli('config','add',json.dumps(script),ok=True)
        self.cli('run','test.input',ok=False)
        text='literal $(do not run)\nΩ'
        self.assertEqual(self.cli('run','test.input','--input',json.dumps({'input':text}),ok=True)['result']['stdout'],text)
        self.assertEqual(self.cli('run','test.input','--input','123',ok=True)['result']['stdout'],'123')
        self.cli('snippets','remove','snippet.test',ok=True)
        self.assertEqual(self.cli('snippets','list',ok=True)['result'],[])
        self.cli('config','set','snippetExpansionEnabled','true',ok=True)
        self.assertTrue(self.cli('config','get',ok=True)['result']['snippetExpansionEnabled'])

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

    def test_new_identity_and_legacy_environment(self):
        self.assertEqual(self.cli('status',ok=True)['result']['name'],'Ruycast')
        env=dict(self.env);env.pop('RUYCAST_HOME',None);env['RUSTCAST_HOME']=str(self.root)
        result=subprocess.run([str(CLI),'status'],env=env,capture_output=True,text=True,check=True)
        self.assertEqual(json.loads(result.stdout)['result']['profile'],str(self.root))
        # Explicit new profile wins over a legacy environment variable.
        env['RUYCAST_HOME']=str(self.root);env['RUSTCAST_HOME']=str(self.root/'unused')
        result=subprocess.run([str(CLI),'status'],env=env,capture_output=True,text=True,check=True)
        self.assertEqual(json.loads(result.stdout)['result']['profile'],str(self.root))

    def test_alias_search_and_dry_run(self):
        r = self.cli('search', 'echo-test', ok=True)
        self.assertEqual(r['result'][0]['id'], 'test.echo')
        before = self.cli('usage')['result']
        self.assertEqual(self.cli('run', 'echo-test', '--dry-run', ok=True)['result']['target'], '/usr/bin/printf')
        self.assertEqual(self.cli('usage')['result'], before)

    def test_script_literal_arguments_and_failure(self):
        r = self.cli('run', 'echo-test', ok=True)
        self.assertEqual(r['result']['stdout'], 'literal $(not a command)\nΩ')
        self.assertEqual(self.cli('run', 'test.fail', ok=False)['exitCode'], 1)
        self.assertGreater(self.cli('usage')['result']['test.echo'], 0)

    def test_script_timeout_and_bounded_output(self):
        start = time.monotonic()
        self.assertIn('timed out', self.cli('run', 'test.timeout', ok=False)['error'])
        self.assertLess(time.monotonic() - start, 2)
        r = self.cli('run', 'test.output', ok=True)
        self.assertEqual(len(r['result']['stdout']), 65536)

    def test_invalid_external_config_retains_working_runtime(self):
        (self.root / 'config.json').write_text('{invalid')
        self.cli('reload', ok=False)
        self.assertEqual(self.cli('search', 'echo-test', ok=True)['result'][0]['id'], 'test.echo')
        self.assertTrue(self.cli('status')['result']['headless'])

    def test_config_set_validation_and_conflict(self):
        original = (self.root / 'config.json').read_bytes()
        self.cli('config', 'set', 'clipboard.limit', '0', ok=False)
        self.assertEqual((self.root / 'config.json').read_bytes(), original)
        self.cli('config', 'set', 'clipboard.limit', '50', '--expect-sha256', 'wrong', ok=False)
        self.assertEqual((self.root / 'config.json').read_bytes(), original)
        self.cli('config', 'set', 'clipboard.limit', '50', ok=True)
        self.assertEqual(self.cli('config', 'get')['result']['clipboard']['limit'], 50)

    def test_concurrent_config_edits_preserve_both_settings(self):
        with ThreadPoolExecutor(max_workers=2) as pool:
            tasks = [pool.submit(self.cli, 'config', 'set', key, value, ok=True) for key, value in [('clipboard.limit', '77'), ('clipboard.retentionDays', '12')]]
            for task in tasks:
                task.result()
        c = self.cli('config', 'get')['result']['clipboard']
        self.assertEqual((c['limit'], c['retentionDays']), (77, 12))

    def test_add_remove_and_duplicate_alias(self):
        command = {'id': 'test.added', 'title': 'Added', 'kind': 'path', 'target': '/', 'aliases': ['added']}
        self.cli('config', 'add', json.dumps(command), ok=True)
        self.assertEqual(self.cli('run', 'added', '--dry-run')['result']['id'], 'test.added')
        self.cli('config', 'add', json.dumps(command), ok=False)
        self.cli('config', 'remove', 'test.added', ok=True)
        self.cli('run', 'added', '--dry-run', ok=False)

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

    def test_unknown_config_field_is_rejected_by_rust_and_native_bridge(self):
        bad = dict(self.config, misspelledSetting=True)
        (self.root / 'config.json').write_text(json.dumps(bad))
        self.cli('config', 'validate', ok=False)
        self.cli('reload', ok=False)
        self.assertEqual(self.cli('search', 'echo-test', ok=True)['result'][0]['id'], 'test.echo')

    def test_graceful_quit_removes_control_socket(self):
        profile = self.root / 'shutdown'
        env = dict(self.env, RUYCAST_HOME=str(profile))
        subprocess.run([str(BINARY), '--init-config'], env=env, capture_output=True, check=True)
        child = subprocess.Popen([str(BINARY), '--headless'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            for _ in range(100):
                if (profile / 'control.sock').exists():
                    break
                time.sleep(.02)
            stop = subprocess.run([str(CLI), 'quit'], env=env, capture_output=True, text=True, timeout=5)
            self.assertTrue(json.loads(stop.stdout)['ok'])
            child.wait(timeout=5)
            self.assertFalse((profile / 'control.sock').exists(), 'Graceful exit must remove the socket before an installer checks shutdown')
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)

    def test_unknown_operation_returns_json_error(self):
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(3)
            client.connect(str(self.root / 'control.sock'))
            client.sendall(b'{"op":"missing"}\n')
            r = json.loads(client.recv(8192))
        self.assertFalse(r['ok'])

    def test_failed_bridge_retains_provider_error_exit_status_and_queryable_log(self):
        bridge=self.root/'personal_bridge.py'
        fixture={'ok':False,'code':'provider_failed','error':'Status writer rejected the note','diagnostics':{'provider':'task-status.sh','providerExitCode':'23','providerStderr':'token='+'sample-secret-value'}}
        bridge.write_text('import sys\nprint('+repr(json.dumps(fixture))+')\nsys.exit(23)\n')
        command={'id':'test.status-diagnostic','title':'Task Done Fixture','kind':'script','target':'/usr/bin/python3','arguments':[str(bridge)]}
        self.cli('config','add',json.dumps(command),ok=True)
        reply=self.cli('run',command['id'],ok=False)
        self.assertEqual(reply['error'],'Status writer rejected the note')
        self.assertEqual(reply.get('exitCode'),23)
        logs=self.cli('logs','--command',command['id'],'--errors',ok=True)['result']
        self.assertEqual(logs[0]['commandID'],command['id'])
        self.assertEqual(logs[0]['exitStatus'],23)
        self.assertEqual(logs[0]['code'],'provider_failed')
        self.assertIn('Status writer rejected',logs[0]['message'])
        self.assertNotIn('sample-secret-value',json.dumps(logs))
        self.assertEqual(reply['diagnosticID'],logs[0]['traceID'])
