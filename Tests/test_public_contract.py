"""Public defaults, build provenance and source selection stay independent of private state."""
import importlib.util,json,os,sys,tempfile,unittest,subprocess,tarfile,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'scripts'))
import build_receipt
spec=importlib.util.spec_from_file_location('public_export',ROOT/'scripts/prepare-public.py');public=importlib.util.module_from_spec(spec);spec.loader.exec_module(public)
class PublicContractTests(unittest.TestCase):
    def test_neutral_defaults_and_structured_contract(self):
        config=json.loads((ROOT/'config/default.json').read_text());text=json.dumps(config)
        self.assertNotIn('Syncthing',text);self.assertNotIn('link.vault',text);self.assertFalse(config['hyperKeyEnabled']);self.assertFalse(config['snippetExpansionEnabled'])
        vision=json.loads((ROOT/'docs/vision.json').read_text());self.assertEqual(vision['artifact_kind'],'repo_design_vision_contract');self.assertTrue({'agent-first','json-first'}<={p['id'] for p in vision['principles']})
    def test_export_excludes_private_state_and_is_self_contained(self):
        names=public.selected(ROOT)
        for name in names:self.assertTrue((ROOT/name).is_file(),name)
        self.assertFalse(any(n.startswith(('.go/','.build/','.git/')) or 'personal_bridge' in n or 'transition_catalog' in n for n in names))
        self.assertIn('Tests/test_config_directory.py',names);self.assertIn('.github/pull_request_template.md',names);self.assertIn('scripts/build_receipt.py',names);self.assertIn('Tests/test_install_migration.py',names);self.assertIn('assets/hero.png',names)
    def test_source_mutation_during_build_cannot_receive_valid_receipt(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get('VIGGO_TEST_ROOT',ROOT/'.build')) as tmp:
            root=Path(tmp);(root/'Sources').mkdir();(root/'Sources/Example.swift').write_text('before')
            before=build_receipt.fingerprint(root);(root/'Sources/Example.swift').write_text('after')
            with self.assertRaisesRegex(RuntimeError,'changed during build'):build_receipt.record(root,before)

    def test_build_and_install_share_exclusive_artifact_lock(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get('VIGGO_TEST_ROOT',ROOT/'.build')) as tmp:
            code="import fcntl,sys; f=open(sys.argv[1],'a'); fcntl.flock(f,fcntl.LOCK_EX|fcntl.LOCK_NB)"
            with build_receipt.artifact_lock(tmp):
                blocked=subprocess.run([sys.executable,'-c',code,str(Path(tmp)/'.build/artifacts.lock')],capture_output=True)
                self.assertNotEqual(blocked.returncode,0)
            available=subprocess.run([sys.executable,'-c',code,str(Path(tmp)/'.build/artifacts.lock')],capture_output=True)
            self.assertEqual(available.returncode,0)

    def test_archive_preserves_patch_version_and_excludes_development_state(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get('VIGGO_TEST_ROOT',ROOT/'.build')) as tmp:
            destination=Path(tmp)/'ruycast-source-0.6.2'
            result=public.prepare(ROOT,destination)
            self.assertEqual(Path(result['archive']).name,'ruycast-source-0.6.2.tar.gz')
            with tarfile.open(result['archive']) as tar:
                names=tar.getnames()
                self.assertFalse(any('/.git/' in n or '/.build/' in n or '/.go/' in n or 'personal_bridge' in n for n in names))
                manifest=json.load(tar.extractfile('ruycast/PUBLIC-SOURCE.json'))
                for name,digest in manifest['files'].items():
                    self.assertEqual(hashlib.sha256(tar.extractfile('ruycast/'+name).read()).hexdigest(),digest,name)

    def test_existing_archive_is_preserved(self):
        with tempfile.TemporaryDirectory(dir=os.environ.get('VIGGO_TEST_ROOT',ROOT/'.build')) as tmp:
            destination=Path(tmp)/'ruycast-source-0.6.2';archive=destination.with_name(destination.name+'.tar.gz');archive.write_bytes(b'user artifact')
            with self.assertRaisesRegex(RuntimeError,'Archive exists'):public.prepare(ROOT,destination)
            self.assertEqual(archive.read_bytes(),b'user artifact');self.assertFalse(destination.exists())
