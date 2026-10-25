"""Unsigned candidates must bind to public source and preserve existing artifacts."""
import importlib.util
import json
import plistlib
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'scripts'))
import build_receipt
spec = importlib.util.spec_from_file_location('package_app', ROOT / 'scripts/package-app.py')
packager = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packager)


class PackageAppTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'public'
        self.root.mkdir()
        source = self.root / 'Sources/Main.swift'
        source.parent.mkdir(); source.write_text('reviewed public source')
        for name in build_receipt.BINARIES:
            p = self.root / name; p.parent.mkdir(parents=True, exist_ok=True); p.write_bytes(name.encode()); p.chmod(0o755)
        build_receipt.record(self.root, build_receipt.fingerprint(self.root))
        manifest = {'schema': 'ruycast.public-source.v1', 'version': '0.7.3',
                    'files': {'Sources/Main.swift': build_receipt.sha(source)}}
        (self.root / 'PUBLIC-SOURCE.json').write_text(json.dumps(manifest))
        self.out = Path(self.temporary.name) / 'candidate'

    def test_candidate_has_only_explicit_bundle_resources_and_no_publication_claim(self):
        (self.root / 'private-profile.json').write_text('secret')
        (self.root / 'scripts').mkdir(); (self.root / 'scripts/personal_bridge.py').write_text('private')
        result = packager.package(self.root, self.out, '26')
        app = self.out / 'Ruycast.app'
        info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
        self.assertEqual(info['CFBundleVersion'], '26'); self.assertEqual(info['CFBundleShortVersionString'], '0.7.3')
        self.assertEqual(info['CFBundleIdentifier'], 'nl.viggomeesters.ruycast')
        self.assertTrue(info['LSUIElement']); self.assertFalse(result['publishable']); self.assertFalse(result['notarized'])
        self.assertEqual(sorted(str(p.relative_to(app)) for p in app.rglob('*') if p.is_file()),
                         ['Contents/Info.plist', 'Contents/MacOS/RuycastApp', 'Contents/Resources/ruycast'])
        self.assertTrue((app / 'Contents/MacOS/RuycastApp').stat().st_mode & 0o111)

    def test_private_checkout_and_missing_manifest_are_rejected_before_staging(self):
        (self.root / '.go').mkdir()
        with self.assertRaisesRegex(RuntimeError, 'clean public'): packager.package(self.root, self.out, '26')
        (self.root / '.go').rmdir(); (self.root / 'PUBLIC-SOURCE.json').unlink()
        with self.assertRaisesRegex(RuntimeError, 'clean public'): packager.package(self.root, self.out, '26')
        self.assertFalse(self.out.exists())

    def test_candidate_verification_rejects_post_staging_mutation_or_extra_file(self):
        packager.package(self.root, self.out, '26')
        self.assertEqual(packager.verify_candidate(self.root, self.out)['build'], '26')
        extra = self.out / 'Ruycast.app/Contents/Resources/private.txt'; extra.write_text('unreviewed')
        with self.assertRaisesRegex(RuntimeError, 'Candidate changed'): packager.verify_candidate(self.root, self.out)
        extra.unlink()
        info = self.out / 'Ruycast.app/Contents/Info.plist'; info.write_bytes(info.read_bytes() + b'changed')
        with self.assertRaisesRegex(RuntimeError, 'Candidate changed'): packager.verify_candidate(self.root, self.out)

    def test_stale_source_or_binary_receipt_cannot_be_packaged(self):
        binary = self.root / build_receipt.BINARIES[0]; binary.write_bytes(b'changed binary')
        with self.assertRaisesRegex(RuntimeError, 'binaries changed'): packager.package(self.root, self.out, '26')
        (self.root / 'Sources/Main.swift').write_text('changed source')
        with self.assertRaisesRegex(RuntimeError, 'Public source changed'): packager.package(self.root, self.out, '26')
        self.assertFalse(self.out.exists())

    def test_existing_artifact_and_symlink_are_preserved(self):
        self.out.mkdir(); marker = self.out / 'human.txt'; marker.write_text('keep')
        with self.assertRaisesRegex(RuntimeError, 'Destination exists'): packager.package(self.root, self.out, '26')
        self.assertEqual(marker.read_text(), 'keep')
        link = self.out.with_name('link'); link.symlink_to(self.out)
        with self.assertRaisesRegex(RuntimeError, 'Destination exists'): packager.package(self.root, link, '26')

    def test_changes_during_copy_do_not_expose_mixed_candidate(self):
        original = packager.shutil.copy2
        def mutate(source, target):
            result = original(source, target)
            (self.root / 'Sources/Main.swift').write_text('concurrent mutation')
            return result
        with patch.object(packager.shutil, 'copy2', side_effect=mutate):
            with self.assertRaisesRegex(RuntimeError, 'Public source changed'): packager.package(self.root, self.out, '26')
        self.assertFalse(self.out.exists())

    def test_linked_source_and_invalid_metadata_are_rejected(self):
        for value in ['0', '-1', '26.1', '../26']:
            with self.assertRaisesRegex(RuntimeError, 'positive integer'): packager.package(self.root, self.out, value)
        source = self.root / 'Sources/Main.swift'; source.unlink(); source.symlink_to('/etc/hosts')
        with self.assertRaisesRegex(RuntimeError, 'linked public source'): packager.package(self.root, self.out, '26')
        self.assertFalse(self.out.exists())
