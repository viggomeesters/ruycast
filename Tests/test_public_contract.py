"""Public defaults, build provenance and source selection stay independent of private state."""
import importlib.util,json,os,sys,tempfile,unittest,subprocess,tarfile,hashlib
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1];sys.path.insert(0,str(ROOT/'scripts'))
import build_receipt,prepare_release,shutil
from unittest.mock import patch
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
        self.assertIn('Tests/test_config_directory.py',names);self.assertIn('.github/pull_request_template.md',names);self.assertIn('scripts/build_receipt.py',names);self.assertIn('scripts/prepare_release.py',names);self.assertIn('Tests/test_install_migration.py',names);self.assertIn('assets/hero.png',names)
    def release_fixture(self,root):
        host=root/prepare_release.HOST;cli=root/prepare_release.CLI
        host.parent.mkdir(parents=True);cli.parent.mkdir(parents=True)
        source=root/'fixture.c';source.write_text('int main(void) {return 0;}\n')
        subprocess.run(['clang','-g',str(source),'-o',str(host)],check=True,capture_output=True)
        shutil.copy2(host,cli)
        subprocess.run(['xcrun','dsymutil',str(host),'-o',str(host)+'.dSYM'],check=True,capture_output=True)
        return host,cli

    def test_release_strip_preserves_real_sections_and_matching_diagnostics(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'.build') as tmp:
            root=Path(tmp);host,cli=self.release_fixture(root)
            before={n:prepare_release.sections(root/n) for n in [prepare_release.HOST,prepare_release.CLI]}
            record=prepare_release.prepare(root)
            for name,sections in before.items():self.assertEqual(prepare_release.sections(root/name),sections)
            self.assertEqual(prepare_release.prepare(root),record) # No repeated stripping/size drift.
            original=root/record['artifacts'][prepare_release.HOST]['unstrippedPath']
            original.write_bytes(original.read_bytes()+b'changed')
            with self.assertRaisesRegex(RuntimeError,'symbols changed'):prepare_release.prepare(root)

    def test_cached_artifacts_keep_their_original_diagnostics_across_partial_rebuilds(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'.build') as tmp:
            root=Path(tmp);host,cli=self.release_fixture(root);first=prepare_release.prepare(root)
            source=root/'changed.c';source.write_text('int main(void) {return 3;}\n')
            subprocess.run(['clang','-g',str(source),'-o',str(host)],check=True,capture_output=True)
            shutil.rmtree(str(host)+'.dSYM')
            subprocess.run(['xcrun','dsymutil',str(host),'-o',str(host)+'.dSYM'],check=True,capture_output=True)
            second=prepare_release.prepare(root)
            self.assertEqual(first['artifacts'][prepare_release.CLI],second['artifacts'][prepare_release.CLI])
            self.assertEqual(first['binaries'][prepare_release.CLI],second['binaries'][prepare_release.CLI])
            subprocess.run(['clang','-g',str(source),'-o',str(cli)],check=True,capture_output=True)
            third=prepare_release.prepare(root)
            self.assertEqual(second['artifacts'][prepare_release.HOST],third['artifacts'][prepare_release.HOST])
            self.assertEqual(second['binaries'][prepare_release.HOST],third['binaries'][prepare_release.HOST])
            prepare_release.verify_symbols(root,third)

    def test_second_replace_failure_and_interrupted_pair_recover_before_retry(self):
        for interrupted in [False,True]:
            with self.subTest(interrupted=interrupted),tempfile.TemporaryDirectory(dir=ROOT/'.build') as tmp:
                root=Path(tmp);host,cli=self.release_fixture(root)
                before={n:build_receipt.sha(root/n) for n in [prepare_release.HOST,prepare_release.CLI]}
                real_replace=os.replace;failed=False
                def replace(source,destination):
                    nonlocal failed
                    if Path(destination)==cli and not failed:
                        failed=True;raise OSError('injected second artifact failure')
                    return real_replace(source,destination)
                with patch.object(prepare_release.os,'replace',replace):
                    if interrupted:
                        # Leave the durable intent as an interrupted process would;
                        # the next call must recover without the in-memory handler.
                        with patch.object(prepare_release,'recover',lambda root:None):
                            with self.assertRaisesRegex(OSError,'second artifact'):prepare_release.prepare(root)
                    else:
                        with self.assertRaisesRegex(OSError,'second artifact'):prepare_release.prepare(root)
                if not interrupted:self.assertEqual({n:build_receipt.sha(root/n) for n in before},before)
                result=prepare_release.prepare(root)
                self.assertEqual({n:r['unstrippedSHA256'] for n,r in result['artifacts'].items()},before)
                self.assertFalse((root/'.build/receipts/release-preparation.pending.json').exists())
                self.assertEqual(prepare_release.prepare(root),result)

    def test_mismatched_dsym_cannot_mutate_build_artifacts(self):
        with tempfile.TemporaryDirectory(dir=ROOT/'.build') as tmp:
            root=Path(tmp);host,cli=self.release_fixture(root)
            before={n:build_receipt.sha(root/n) for n in [prepare_release.HOST,prepare_release.CLI]}
            # A second real linked image has a different UUID, not a mocked tool reply.
            source=root/'other.c';source.write_text('int main(void) {return 7;}\n');other=root/'other'
            subprocess.run(['clang','-g',str(source),'-o',str(other)],check=True,capture_output=True)
            shutil.rmtree(str(host)+'.dSYM')
            subprocess.run(['xcrun','dsymutil',str(other),'-o',str(host)+'.dSYM'],check=True,capture_output=True)
            with self.assertRaisesRegex(RuntimeError,'dSYM does not match'):prepare_release.prepare(root)
            self.assertEqual({n:build_receipt.sha(root/n) for n in before},before)

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
