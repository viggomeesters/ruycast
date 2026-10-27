"""Architecture boundaries guard production modules, not a duplicate implementation."""
import re, unittest
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
