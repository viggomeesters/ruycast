#!/usr/bin/env python3
"""Own-process NSWorkspace ABI tests. All four SDK selectors are replaced;
no real application, URL or file is opened and no personal inventory is read.
"""
import json
import os
from pathlib import Path
import subprocess
import shutil
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get('RUYCAST_WORKSPACE_PROBE', ROOT / 'target/debug/workspace-probe'))
SHIM = ROOT / 'target/workspace-shim.dylib'


def request(kind='app', target='dev.fixture.success', **kwargs):
    return dict(kind=kind, target=target, commandId='fixture', title='Fixture', **kwargs)


class Workspace(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        target = Path(os.environ.get('CARGO_TARGET_DIR', ROOT.parent / '.build/full-rust-release/native'))
        artifact = Path(os.environ.get('RUYCAST_TEST_WORKSPACE_SHIM', target / 'release/examples/libworkspace_fixture.dylib'))
        if not artifact.is_absolute() or not artifact.is_file():
            raise RuntimeError('Build --features test-tools --example workspace-fixture first')
        shutil.copyfile(artifact, SHIM)

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


if __name__ == '__main__':
    unittest.main(verbosity=2)
