"""Real disposable Rust processes: filesystem publication and config/model flow."""
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PROBE = ROOT / 'target/debug/foundation-probe'


class FoundationProcesses(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not PROBE.is_file():
            raise RuntimeError('Build with --features test-tools --bin foundation-probe first')
        cls.library = tempfile.TemporaryDirectory(prefix='foundation-shim-', dir=ROOT / 'target')
        cls.shim = Path(cls.library.name) / 'fsync.dylib'
        subprocess.run(['clang', '-dynamiclib', '-Wall', '-Wextra',
                        str(ROOT / 'tests/fcntl_sync_shim.c'), '-o', str(cls.shim)],
                       check=True, capture_output=True)

    @classmethod
    def tearDownClass(cls):
        cls.library.cleanup()

    def run_fixture(self, mode, inject=None, mutation=None, portable_acl=False):
        folder = tempfile.TemporaryDirectory(prefix='foundation-process-', dir=ROOT / 'target')
        self.addCleanup(folder.cleanup)
        base = Path(folder.name).resolve()
        profile = base / 'profile'
        profile.mkdir(mode=0o700)
        config = (ROOT.parent / 'config/default.json').read_bytes()
        (profile / 'config.json').write_bytes(config)
        (profile / 'config.json').chmod(0o600)
        parent_acl = None
        if portable_acl:
            portable = base / 'portable'
            portable.mkdir(mode=0o755)
            portable.chmod(0o755)
            subprocess.run(['/bin/chmod', '+a', 'everyone allow read,execute,readattr,readextattr,readsecurity,file_inherit,directory_inherit', str(portable)], check=True, capture_output=True)
            parent_acl = subprocess.check_output(['/bin/ls', '-lde', str(portable)], text=True).splitlines()[1:]
            (portable / 'config.json').write_bytes(config)
            (profile / 'config-location.json').write_text(json.dumps({'directory': str(portable)}))
        env = dict(os.environ)
        for key in list(env):
            if key.startswith('RUYCAST_TEST_FSYNC_') or key == 'DYLD_INSERT_LIBRARIES':
                env.pop(key)
        if inject:
            env.update(DYLD_INSERT_LIBRARIES=str(self.shim), RUYCAST_TEST_FSYNC_DIRECTORY=str(base / inject),
                       RUYCAST_TEST_FSYNC_MARKER=str(base / 'injected'))
            if mutation:
                env['RUYCAST_TEST_FSYNC_MUTATE'] = mutation
                if mutation == 'selector':
                    external = base / 'external'
                    external.mkdir(mode=0o700)
                    (external / 'config.json').write_bytes(config)
                    env['RUYCAST_TEST_SELECTOR_JSON'] = json.dumps({'directory': str(external)})
                if mutation == 'target':
                    env['RUYCAST_TEST_MUTATION_TARGET'] = str(base / 'portable/config.json')
        run = subprocess.run([str(PROBE), mode, str(base)], capture_output=True, text=True, env=env, timeout=10, umask=0o022)
        self.assertEqual(run.returncode, 0, run.stderr)
        if inject:
            self.assertEqual((base / 'injected').read_bytes(), b'injected')
        return base, json.loads(run.stdout), parent_acl

    def test_inline_followup_error_preserves_the_real_postpublication_warning(self):
        _,r,_=self.run_fixture('inline-followup-warning',inject='profile');self.assertTrue(r['warning']);self.assertIn('removed',r['error']);self.assertFalse(r['opened']);self.assertFalse(r['draftRetained']);self.assertEqual(r['aliases'],['published-inline']);self.assertTrue(r['fileEqualsSnapshot'])
    def test_inline_followup_cancel_preserves_the_real_postpublication_warning(self):
        _,r,_=self.run_fixture('inline-cancel-warning',inject='profile');self.assertTrue(r['warning']);self.assertIsNone(r['error']);self.assertFalse(r['opened']);self.assertFalse(r['draftRetained']);self.assertEqual(r['aliases'],['published-inline']);self.assertTrue(r['fileEqualsSnapshot'])
    def test_new_command_fsync_warning_preserves_published_state(self):
        _,r,_=self.run_fixture('new-command',inject='profile');self.assertTrue(r['ok']);self.assertFalse(r['durable']);self.assertTrue(r['warning']);self.assertEqual(r['countAdded'],1);self.assertFalse(r['draftRetained']);self.assertTrue(r['fileEqualsSnapshot'])
    def test_new_command_external_postpublish_edit_is_not_overwritten(self):
        base,r,_=self.run_fixture('new-command',inject='profile',mutation='replace');self.assertFalse(r['ok']);self.assertTrue(r['draftRetained']);self.assertFalse(r['fileEqualsSnapshot']);self.assertEqual((base/'profile/config.json').read_text(),'external fixture edit');self.assertTrue((base/'profile/published.copy').exists())
    def test_prefill_uses_only_a_native_named_disposable_pasteboard(self):
        _,r,_=self.run_fixture('clipboard-prefill');self.assertEqual(r['prefill'],'https://example.com/path');self.assertTrue(r['emptyIgnored']);self.assertTrue(r['largeTextRetained']);self.assertFalse(r['generalPasteboardRead'])

    def test_actual_configuration_reload_updates_only_valid_catalogs(self):
        _, result, _ = self.run_fixture('state')
        self.assertTrue(result['ok'])
        self.assertTrue(result['lastGoodCatalogRetained'])
        self.assertTrue(result['selectedRemovedWithoutReplacement'])

    def test_successful_choice_refreshes_ranking_without_stealing_selection(self):
        _, result, _ = self.run_fixture('ranking')
        self.assertTrue(result['immediateRankRefresh'])
        self.assertTrue(result['selectedIdentityPreserved'])
        self.assertTrue(result['actionsPreserved'])

    def test_binding_save_after_directory_sync_failure_keeps_published_state_and_warning(self):
        base,result,_=self.run_fixture('binding',inject='profile')
        self.assertTrue(result['ok']);self.assertFalse(result['durable']);self.assertTrue(result['warning']);self.assertEqual(result['aliases'],['changed-alias']);self.assertTrue(result['fileEqualsSnapshot']);self.assertFalse(result['draftRetained'])
    def test_disable_after_directory_sync_failure_keeps_explicit_warning(self):
        base,result,_=self.run_fixture('disable',inject='profile')
        self.assertTrue(result['ok']);self.assertFalse(result['durable']);self.assertTrue(result['warning']);self.assertFalse(result['enabled']);self.assertTrue(result['fileEqualsSnapshot'])
    def test_binding_postpublication_external_edit_keeps_external_data_and_draft(self):
        base,result,_=self.run_fixture('binding',inject='profile',mutation='replace')
        self.assertFalse(result['ok']);self.assertTrue(result['draftRetained']);self.assertFalse(result['fileEqualsSnapshot']);self.assertEqual((base/'profile/config.json').read_text(),'external fixture edit');self.assertEqual(json.loads((base/'profile/published.copy').read_text())['commands'][0]['aliases'],['changed-alias'])
    def test_directory_fsync_failure_adopts_published_save_with_warning(self):
        base, result, _ = self.run_fixture('save', inject='profile')
        self.assertTrue(result['ok'])
        self.assertFalse(result['durable'])
        self.assertTrue(result['warning'])
        self.assertEqual(result['activeLimit'], 51)
        self.assertEqual(result['revision'], 1)
        self.assertTrue(result['fileEqualsSnapshot'])
        self.assertEqual(json.loads((base / 'profile/config.json').read_bytes())['clipboard']['limit'], 51)

    def test_external_replace_after_publish_is_not_adopted_or_overwritten(self):
        base, result, _ = self.run_fixture('save', inject='profile', mutation='replace')
        self.assertFalse(result['ok'])
        self.assertEqual(result['revision'], 0)
        self.assertEqual(result['activeLimit'], 200)
        self.assertEqual((base / 'profile/config.json').read_bytes(), b'external fixture edit')
        self.assertEqual(json.loads((base / 'profile/published.copy').read_bytes())['clipboard']['limit'], 51)

    def test_destination_fsync_failure_cleans_only_owned_copy(self):
        _, result, _ = self.run_fixture('relocate', inject='portable')
        self.assertFalse(result['ok'])
        self.assertTrue(result['sourcePreserved'])
        self.assertFalse(result['targetExists'])
        self.assertFalse(result['selectorExists'])

    def test_destination_replacement_is_preserved_on_failed_move(self):
        base, result, _ = self.run_fixture('relocate', inject='portable', mutation='replace')
        self.assertFalse(result['ok'])
        self.assertTrue(result['sourcePreserved'])
        self.assertTrue(result['targetExists'])
        self.assertFalse(result['selectorExists'])
        self.assertEqual((base / 'portable/config.json').read_bytes(), b'external fixture edit')

    def test_published_selector_fsync_failure_switches_to_actual_target(self):
        base, result, _ = self.run_fixture('relocate', inject='profile')
        self.assertTrue(result['ok'])
        self.assertFalse(result['durable'])
        self.assertTrue(result['warning'])
        self.assertTrue(result['activeTarget'])
        self.assertTrue(result['sourcePreserved'])
        self.assertEqual(json.loads((base / 'profile/config-location.json').read_text())['directory'], str(base / 'portable'))

    def test_portable_parent_acl_and_mode_remain_while_new_payload_is_private(self):
        base, result, parent_acl = self.run_fixture('save', portable_acl=True)
        self.assertTrue(result['ok'])
        portable = base / 'portable'
        self.assertEqual(stat.S_IMODE(portable.stat().st_mode), 0o755)
        self.assertEqual(subprocess.check_output(['/bin/ls', '-lde', str(portable)], text=True).splitlines()[1:], parent_acl)
        target = portable / 'config.json'
        self.assertEqual(stat.S_IMODE(target.stat().st_mode), 0o600)
        self.assertEqual(len(subprocess.check_output(['/bin/ls', '-le', str(target)], text=True).splitlines()), 1)

    def test_selector_changed_during_save_is_not_falsely_adopted(self):
        base, result, _ = self.run_fixture('save', inject='profile', mutation='selector')
        self.assertFalse(result['ok'])
        self.assertEqual(result['revision'], 0)
        self.assertEqual(result['activeLimit'], 200)
        self.assertEqual(json.loads((base / 'profile/config-location.json').read_text())['directory'], str(base / 'external'))

    def test_selector_changed_after_move_publication_keeps_recovery_copy(self):
        base, result, _ = self.run_fixture('relocate', inject='profile', mutation='selector')
        self.assertFalse(result['ok'])
        self.assertFalse(result['activeTarget'])
        self.assertTrue(result['sourcePreserved'])
        self.assertTrue(result['targetExists'])
        self.assertEqual(json.loads((base / 'profile/config-location.json').read_text())['directory'], str(base / 'external'))

    def test_target_changed_after_selector_publication_is_preserved(self):
        base, result, _ = self.run_fixture('relocate', inject='profile', mutation='target')
        self.assertFalse(result['ok'])
        self.assertTrue(result['sourcePreserved'])
        self.assertFalse(result['activeTarget'])
        self.assertEqual((base / 'portable/config.json').read_bytes(), b'external fixture edit')


if __name__ == '__main__':
    unittest.main(verbosity=2)
