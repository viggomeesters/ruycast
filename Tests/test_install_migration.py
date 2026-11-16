"""Exercise real migration/rollback filesystem writes in a fake home and app tree."""
import os
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest

REPO=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(REPO/'scripts'))
from install_support import OLD_ID,require_owned_bundle

class MigrationTests(unittest.TestCase):
    def setUp(self):
        base=Path(os.environ.get('VIGGO_TEST_ROOT',REPO/'.build/test-profiles'));base.mkdir(parents=True,exist_ok=True)
        self.tmp=tempfile.TemporaryDirectory(dir=base);self.root=Path(self.tmp.name)
    def tearDown(self):self.tmp.cleanup()




    def test_wrong_bundle_identity_refused(self):
        app=self.root/'Rustcast.app';(app/'Contents').mkdir(parents=True)
        (app/'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier':'org.unrelated.rustcast'}))
        with self.assertRaisesRegex(RuntimeError,'Unrelated app identity'):require_owned_bundle(app,[OLD_ID])
        self.assertTrue(app.exists())
