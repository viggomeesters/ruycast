"""Exercise real migration/rollback filesystem writes in a fake home and app tree."""
import os
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest

REPO=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(REPO/'scripts'))
from install_support import OLD_ID,profile_paths,migrate_profile,check_profile_paths,set_cli_link,require_owned_bundle,require_owned_link

class MigrationTests(unittest.TestCase):
    def setUp(self):
        base=Path(os.environ.get('VIGGO_TEST_ROOT',REPO/'.build/test-profiles'));base.mkdir(parents=True,exist_ok=True)
        self.tmp=tempfile.TemporaryDirectory(dir=base);self.root=Path(self.tmp.name)
    def tearDown(self):self.tmp.cleanup()
    def test_external_local_adapter_stages_without_entering_public_source(self):
        from install_support import preserve_personal_bridge
        repo=self.root/'public';installed=self.root/'installed';stage=self.root/'stage'
        (stage/'Contents/Resources').mkdir(parents=True)
        adapter=self.root/'local-adapter.py';adapter.write_bytes(b'fixture local adapter')
        preserve_personal_bridge(repo,installed,stage,str(adapter))
        self.assertEqual((stage/'Contents/Resources/personal_bridge.py').read_bytes(),adapter.read_bytes())
        self.assertFalse((repo/'scripts/personal_bridge.py').exists())
        with self.assertRaisesRegex(RuntimeError,'absolute regular'):
            preserve_personal_bridge(repo,installed,stage,'relative.py')

    def test_public_source_update_preserves_existing_local_adapter(self):
        from install_support import preserve_personal_bridge
        repo=self.root/'public';installed=self.root/'Ruycast.app';stage=self.root/'stage'
        resource=installed/'Contents/Resources';resource.mkdir(parents=True)
        (stage/'Contents/Resources').mkdir(parents=True)
        adapter=resource/'personal_bridge.py';adapter.write_bytes(b'private adapter fixture')
        preserve_personal_bridge(repo,installed,stage)
        self.assertEqual((stage/'Contents/Resources/personal_bridge.py').read_bytes(),adapter.read_bytes())
        adapter.unlink();adapter.symlink_to(self.root/'external')
        with self.assertRaisesRegex(RuntimeError,'Linked personal adapter'):
            preserve_personal_bridge(repo,installed,stage)


    def test_conflicting_profiles_and_links_are_preserved(self):
        old,new=profile_paths(self.root);old.mkdir(parents=True);new.mkdir()
        (old/'config.json').write_text('old');(new/'config.json').write_text('new')
        with self.assertRaisesRegex(RuntimeError,'Both profiles'):migrate_profile(old,new)
        self.assertEqual((old/'config.json').read_text(),'old');self.assertEqual((new/'config.json').read_text(),'new')
        link=self.root/'ruycast';link.write_text('user executable')
        with self.assertRaisesRegex(RuntimeError,'non-symlink'):set_cli_link(link,'/some/target')
        self.assertEqual(link.read_text(),'user executable')
    def test_unrelated_cli_symlink_is_not_hijacked(self):
        other=self.root/'other-tool';other.write_text('user executable')
        link=self.root/'ruycast';link.symlink_to(other)
        with self.assertRaisesRegex(RuntimeError,'Unrelated CLI symlink'):set_cli_link(link,self.root/'Ruycast.app/Contents/Resources/ruycast')
        self.assertEqual(link.resolve(),other)
        self.assertEqual(other.read_text(),'user executable')

    def test_wrong_bundle_identity_refused(self):
        app=self.root/'Rustcast.app';(app/'Contents').mkdir(parents=True)
        (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':'org.unrelated.rustcast'}))
        with self.assertRaisesRegex(RuntimeError,'Unrelated app identity'):require_owned_bundle(app,[OLD_ID])
        self.assertTrue(app.exists())
