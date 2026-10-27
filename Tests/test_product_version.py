"""Verify one compiled product version reaches source export and rejects drift safely."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/"scripts"))
from product_version import VERSION_SOURCE,read_product_version
spec=importlib.util.spec_from_file_location("version_public_export",ROOT/"scripts/prepare-public.py")
public=importlib.util.module_from_spec(spec);spec.loader.exec_module(public)

class ProductVersionTests(unittest.TestCase):
    def setUp(self):
        base=Path(os.environ.get("VIGGO_TEST_ROOT",ROOT/".build"));base.mkdir(parents=True,exist_ok=True)
        self.temp=tempfile.TemporaryDirectory(dir=base);self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name)/"source";self.root.mkdir()
    def write_version(self,text):
        path=self.root/VERSION_SOURCE;path.parent.mkdir(parents=True,exist_ok=True);path.write_text(text);return path
    def declaration(self,version):
        return 'public enum ProductVersion {\n    public static let current = "'+version+'"\n}\n'
    def public_fixture(self):
        for name in public.selected(ROOT):
            target=self.root/name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/name,target)
    def test_reader_and_actual_swift_compiler_agree_after_one_version_edit(self):
        source=(ROOT/VERSION_SOURCE).read_text();current=read_product_version(ROOT)
        for version in ["0.0.1","17.23.409"]:
            with self.subTest(version=version):
                path=self.write_version(source.replace('"'+current+'"','"'+version+'"'))
                main=self.root/"main.swift";main.write_text('print(ProductVersion.current)\n')
                binary=self.root/"version-probe"
                compiled=subprocess.run(["swiftc",str(path),str(main),"-o",str(binary)],capture_output=True,text=True,timeout=60)
                self.assertEqual(compiled.returncode,0,compiled.stderr)
                self.assertEqual(subprocess.check_output([str(binary)],text=True).strip(),version)
                self.assertEqual(read_product_version(self.root),version)
    def test_missing_linked_ambiguous_or_nonliteral_version_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError,"Cannot read"):read_product_version(self.root)
        values=[self.declaration("01.2.3"),self.declaration("1.2"),self.declaration("1.2.3-beta"),self.declaration("invalid"),self.declaration("1.2.3")+self.declaration("4.5.6"),self.declaration("1.2.3").replace('"1.2.3"','String("1.2.3")')]
        for text in values:
            with self.subTest(text=text):
                self.write_version(text)
                with self.assertRaisesRegex(RuntimeError,"one canonical"):read_product_version(self.root)
        path=self.root/VERSION_SOURCE;path.unlink();target=self.root/"external.swift";target.write_text(self.declaration("1.2.3"));path.symlink_to(target)
        with self.assertRaisesRegex(RuntimeError,"regular source"):read_product_version(self.root)
    def test_export_follows_changed_compiled_version(self):
        self.public_fixture();self.write_version(self.declaration("17.23.409"))
        destination=Path(self.temp.name)/"export"
        result=public.prepare(self.root,destination)
        manifest=json.loads((destination/"PUBLIC-SOURCE.json").read_text())
        self.assertEqual(manifest["version"],"17.23.409")
        self.assertEqual(read_product_version(destination),"17.23.409")
        self.assertIn("scripts/product_version.py",manifest["files"])
        self.assertIn("Tests/test_product_version.py",manifest["files"])
        self.assertTrue(Path(result["archive"]).is_file())
    def test_invalid_version_creates_no_export_or_archive(self):
        self.public_fixture();self.write_version(self.declaration("not-a-version"))
        destination=Path(self.temp.name)/"export"
        with self.assertRaisesRegex(RuntimeError,"one canonical"):public.prepare(self.root,destination)
        self.assertFalse(destination.exists());self.assertFalse(destination.with_name(destination.name+".tar.gz").exists())

    def test_nonregular_source_is_rejected_without_waiting_for_a_writer(self):
        path=self.root/VERSION_SOURCE;path.parent.mkdir(parents=True);path.mkdir()
        with self.assertRaisesRegex(RuntimeError,"regular source"):read_product_version(self.root)
        path.rmdir();os.mkfifo(path)
        code="from product_version import read_product_version;import sys;read_product_version(sys.argv[1])"
        result=subprocess.run([sys.executable,"-c",code,str(self.root)],env={**os.environ,"PYTHONPATH":str(ROOT/"scripts")},capture_output=True,text=True,timeout=2)
        self.assertNotEqual(result.returncode,0)
        self.assertIn("regular source",result.stderr)
