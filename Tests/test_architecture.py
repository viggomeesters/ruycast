"""Architecture boundaries guard production modules, not a duplicate implementation."""
import re, subprocess, tempfile, unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
class ArchitectureTests(unittest.TestCase):
    def test_portable_models_have_no_platform_or_runtime_dependency(self):
        files=list((ROOT/'Sources/LauncherModels').rglob('*.swift'))
        self.assertTrue(files)
        for path in files:
            imports=set(re.findall(r'^import (\w+)',path.read_text(),re.M))
            self.assertLessEqual(imports,{'Foundation'},str(path))
            self.assertNotRegex(path.read_text(),r'\b(?:FileManager|URLSession|Process|NSWorkspace|RustCore)\b',str(path))
    def test_swift_uses_imported_c_symbols(self):
        for path in (ROOT/'Sources').rglob('*.swift'):
            self.assertNotIn('@_silgen_name',path.read_text(),str(path))
        self.assertTrue((ROOT/'Sources/CRuycastCore/include/ruycast_core.h').is_file())

    def test_main_actor_and_worker_boundaries_are_compiler_checked(self):
        # Typechecking imports the actual debug module without executing its
        # entry point or touching the user's profile. Positive control prevents
        # a missing module/toolchain from masquerading as a rejected violation.
        def compile(body):
            with tempfile.TemporaryDirectory(prefix='rc-isolation-') as directory:
                source=Path(directory)/'probe.swift'
                source.write_text('@testable import RuycastApp\n'+body+'\n')
                return subprocess.run([
                    'swiftc','-swift-version','6','-typecheck',
                    '-I',str(ROOT/'.build/swift/debug/Modules'),
                    '-Xcc','-fmodule-map-file='+str(ROOT/'.build/swift/debug/CRuycastCore.build/module.modulemap'),
                    '-Xcc','-I','-Xcc',str(ROOT/'Sources/CRuycastCore/include'),str(source)
                ],text=True,capture_output=True,timeout=60)
        good=compile('@MainActor func allowed(_ state:AppState) {state.query="valid";let _:Int=ClipboardWork().sync {42}}')
        self.assertEqual(good.returncode,0,good.stderr)
        for body in [
            'func forbidden(_ state:AppState) {state.query="invalid"}',
            '@MainActor func forbidden(_ state:AppState) {let _=ClipboardWork().sync {state.query}}'
        ]:
            rejected=compile(body)
            self.assertNotEqual(rejected.returncode,0)
            self.assertIn("main actor-isolated property 'query'",rejected.stderr)
