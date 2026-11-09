#!/usr/bin/env python3
"""Own-process NSWorkspace ABI tests. All four SDK selectors are replaced;
no real application, URL or file is opened and no personal inventory is read.
"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / 'target/debug/workspace-probe'
SHIM = ROOT / 'target/workspace-shim.dylib'


def request(kind='app', target='dev.fixture.success', **kwargs):
    return dict(kind=kind, target=target, commandId='fixture', title='Fixture', **kwargs)


class Workspace(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', '-fobjc-arc', '-fblocks',
                        '-dynamiclib', '-framework', 'AppKit',
                        str(ROOT / 'tests/workspace_shim.m'), '-o', str(SHIM)], check=True,
                       capture_output=True)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='workspace-', dir=ROOT / 'target')
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name).resolve()
        self.home.chmod(0o700)
        (self.home / '.ruycast-test-fixture').write_text('own-disposable-fixture-v1')

    def probe(self, cases, *, shim=True, **options):
        env = dict(os.environ)
        for key in list(env):
            if key.startswith(('RUYCAST_', 'RUSTCAST_')) or key == 'DYLD_INSERT_LIBRARIES':
                env.pop(key)
        env.update(HOME=str(self.home), RUYCAST_WORKSPACE_FIXTURE_HOME=str(self.home))
        if shim:
            env['DYLD_INSERT_LIBRARIES'] = str(SHIM)
        run = subprocess.run([str(BIN)], input=json.dumps(dict(cases=cases, **options)),
                             text=True, capture_output=True, env=env, timeout=12)
        self.assertEqual(run.returncode, 0, run.stderr + run.stdout)
        result = json.loads(run.stdout)
        self.assertTrue(result['ok'])
        self.assertFalse(result['ownApplicationActive'])
        self.assertEqual(result['ownWindowCount'], 0)
        trace = self.home / 'workspace-trace.jsonl'
        self.trace = [json.loads(line) for line in trace.read_text().splitlines()] if trace.exists() else []
        if shim and self.trace:
            self.assertEqual(self.trace[0]['phase'], 'installed')
            self.assertEqual(self.trace[0]['selectors'], 4)
        return result

    def one(self, req=None, **options):
        return self.probe([dict(request=req or request(), **options)])['cases'][0]

    def opens(self):
        return [entry for entry in self.trace if entry['phase'].startswith('open-')]

    def unknown(self, value):
        self.assertTrue(value['committed'])
        self.assertFalse(value['response']['ok'])
        self.assertTrue(value['response']['actionCompleted'])
        self.assertEqual(value['response']['code'], 'open-outcome-unknown')

    def test_app_callback_from_worker_and_exact_response(self):
        value = self.one()
        self.assertEqual(value['response'], dict(ok=True, result=dict(command='fixture', state='opened')))
        self.assertEqual(value['beforeCommitCalls'], 1)
        self.assertEqual([x['phase'] for x in self.trace], ['installed', 'resolve', 'open-app', 'callback'])
        self.assertTrue(all(not x['mainThread'] for x in self.trace[1:]))

    def test_missing_and_invalid_application_never_commit(self):
        data = self.probe([dict(request=request(target='dev.fixture.' + mode))
                           for mode in ['missing', 'invalid-location']])
        for value in data['cases']:
            self.assertFalse(value['response']['ok'])
            self.assertFalse(value['committed'])
            self.assertEqual(value['beforeCommitCalls'], 0)
        self.assertEqual(self.opens(), [])
        self.assertEqual(data['cases'][0]['response']['error'], 'App is not installed: dev.fixture.missing')

    def test_default_url_uses_synchronous_boolean_and_encoding(self):
        value = self.one(request('url', 'ruycast-fixture://success/Ω space?q=hello world'))
        self.assertTrue(value['response']['ok'])
        self.assertEqual(value['pendingCallbacks'], 0)
        self.assertEqual(self.opens()[0]['phase'], 'open-default')
        self.assertIn('%20', self.opens()[0]['url'])

    def test_default_url_known_false_is_not_unknown(self):
        value = self.one(request('url', 'ruycast-fixture://reject/path'))
        self.assertTrue(value['committed'])
        self.assertEqual(value['response'], dict(ok=False, error='macOS could not open Fixture.'))

    def test_path_unicode_newline_literal_and_missing(self):
        path = self.home / 'Ω file\n$(literal)'
        path.write_text('fixture')
        data = self.probe([dict(request=request('path', str(path))),
                           dict(request=request('path', str(self.home / 'missing')))])
        self.assertTrue(data['cases'][0]['response']['ok'])
        self.assertIn('%0A', self.opens()[0]['url'])
        self.assertFalse(data['cases'][1]['committed'])
        self.assertEqual(data['cases'][1]['beforeCommitCalls'], 0)
        self.assertIn('Path does not exist:', data['cases'][1]['response']['error'])

    def test_explicit_open_with_preserves_state_only_shape(self):
        value = self.one(request('url', 'https://example.invalid/a', applicationId='dev.fixture.success'))
        self.assertEqual(value['response'], dict(ok=True, result=dict(state='opened')))
        targets = [e for e in self.trace if e['phase'] == 'open-with-targets']
        self.assertEqual(targets[0]['urls'], ['https://example.invalid/a'])
        self.assertEqual(self.opens()[-1]['phase'], 'open-with')

    def test_missing_open_with_never_commit(self):
        value = self.one(request('url', 'https://example.invalid', applicationId='dev.fixture.missing'))
        self.assertFalse(value['committed'])
        self.assertEqual(value['response']['error'], 'The selected app is not installed. Choose another app in Settings.')
        self.assertEqual(self.opens(), [])

    def test_native_error_and_bounded_large_error(self):
        data = self.probe([dict(request=request(target='dev.fixture.' + mode)) for mode in ['error', 'large-error']])
        small, large = data['cases']
        self.assertEqual(small['response'], dict(ok=False, error='Fixture native error.'))
        self.assertTrue(large['committed'])
        self.assertLess(len(large['response']['error'].encode()), 65536)
        self.assertTrue(large['response']['error'].endswith('…'))

    def test_nil_callback_result_is_unknown(self):
        self.unknown(self.one(request(target='dev.fixture.nil-result')))

    def test_reentrant_and_duplicate_first_callback_wins(self):
        data = self.probe([dict(request=request(target='dev.fixture.' + mode)) for mode in ['inline', 'duplicate']])
        self.assertTrue(all(v['response']['ok'] for v in data['cases']))
        self.assertEqual(len([e for e in self.trace if e['phase'] == 'callback']), 3)

    def test_precommit_rejection_has_no_open(self):
        value = self.one(rejectBeforeCommit=True)
        self.assertEqual(value['beforeCommitCalls'], 1)
        self.assertFalse(value['committed'])
        self.assertEqual(self.opens(), [])

    def test_preexpired_request_does_not_even_resolve(self):
        value = self.one(timeoutMs=0)
        self.assertFalse(value['committed'])
        self.assertEqual(len(self.trace), 1)

    def test_blocked_resolution_is_not_claimed_as_hard_deadline(self):
        value = self.one(request(target='dev.fixture.slow-resolve'), timeoutMs=75)
        self.assertFalse(value['committed'])
        self.assertEqual(value['beforeCommitCalls'], 0)
        self.assertGreaterEqual(value['seconds'], .15)
        self.assertEqual([e['phase'] for e in self.trace], ['installed', 'resolve'])

    def test_cancellation_after_sdk_entry_retires_late_completion(self):
        data = self.probe([dict(request=request(target='dev.fixture.slow'), cancelAfterOpen=True)], settleMs=300)
        self.unknown(data['cases'][0])
        self.assertEqual(data['afterRelease'], 0)
        self.assertEqual([e['phase'] for e in self.trace], ['installed', 'resolve', 'open-app', 'callback'])

    def test_callback_timeout_retires_late_completion(self):
        # Warm NSWorkspace in this same process to isolate the callback deadline.
        data = self.probe([dict(request=request()), dict(request=request(target='dev.fixture.slow'), timeoutMs=75)], settleMs=300)
        self.assertTrue(data['cases'][0]['response']['ok'])
        self.unknown(data['cases'][1])
        self.assertEqual(data['afterRelease'], 0)

    def test_retained_callbacks_bound_capacity_until_sdk_releases(self):
        cases = [dict(request=request())] + [dict(request=request(target='dev.fixture.hold'), timeoutMs=20) for _ in range(17)]
        data = self.probe(cases, releaseCallbacks=True)
        for value in data['cases'][1:17]:
            self.unknown(value)
        blocked = data['cases'][17]
        self.assertFalse(blocked['committed'])
        self.assertEqual(blocked['beforeCommitCalls'], 0)
        self.assertIn('busy', blocked['response']['error'])
        self.assertEqual(data['beforeRelease'], 16)
        self.assertEqual(data['afterRelease'], 0)
        self.assertEqual(len([e for e in self.trace if e['phase'] == 'open-app']), 17)

    def test_missing_interposer_fails_closed(self):
        data = self.probe([dict(request=request())], shim=False)
        self.assertFalse(data['cases'][0]['committed'])
        self.assertIn('boundary', data['cases'][0]['response']['error'])
        self.assertEqual(self.trace, [])

    def test_invalid_marker_fails_closed(self):
        (self.home / '.ruycast-test-fixture').write_text('wrong')
        data = self.probe([dict(request=request())])
        self.assertFalse(data['cases'][0]['committed'])
        self.assertEqual(self.trace, [])

    def test_fixture_denies_real_targets_and_external_paths(self):
        (self.home / 'escape').symlink_to('/Applications')
        cases = [request(target='com.apple.finder'), request('url', 'https://apple.com'),
                 request('path', '/Applications'), request('path', str(self.home / 'escape/Safari.app')),
                 request('path', str(self.home / '../outside')), request('url', 'http://[bad')]
        data = self.probe([dict(request=r) for r in cases])
        self.assertTrue(all(not c['committed'] and not c['response']['ok'] for c in data['cases']))
        self.assertEqual(self.opens(), [])

    def test_nul_rejected_before_native_resolution(self):
        value = self.one(request(target='dev.fixture.\0bad'))
        self.assertFalse(value['committed'])
        self.assertEqual(len(self.trace), 1)

    def test_file_url_and_resolved_app_urls_cannot_escape_fixture(self):
        (self.home / 'escape').symlink_to('/Applications')
        cases = [request('url', (self.home / 'escape/Fake.app').as_uri()),
                 request('url', (self.home / '../outside').as_uri()),
                 request(target='dev.fixture.x/../../../outside'),
                 request('url', 'https://example.invalid', applicationId='dev.fixture.x/../../../outside')]
        data = self.probe([dict(request=r) for r in cases])
        self.assertTrue(all(not c['committed'] and not c['response']['ok'] for c in data['cases']))
        self.assertEqual(self.opens(), [])

    def test_panic_before_and_after_commit_preserves_truth_and_releases_callbacks(self):
        data = self.probe([dict(request=request(), panicBeforeCommit=True),
                           dict(request=request(target='dev.fixture.slow'), panicAfterOpen=True)], settleMs=300)
        self.assertFalse(data['cases'][0]['committed'])
        self.unknown(data['cases'][1])
        self.assertEqual(data['afterRelease'], 0)
        self.assertEqual(len([e for e in self.trace if e['phase'] == 'open-app']), 1)


if __name__ == '__main__':
    unittest.main(verbosity=2)
