"""Policy rejects real source drift; all mutations are disposable fixtures."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("rust_only", ROOT / "scripts/check_rust_only.py")
policy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(policy)


class RustOnly(unittest.TestCase):
    def fixture(self):
        (ROOT / "target").mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="rust-only-policy-", dir=ROOT / "target")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        for path in ["Sources/Legacy.swift", "Package.swift", "rust/src/lib.rs",
                     "rust-native/src/main.rs"]:
            file = root / path
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text("// own policy fixture\n")
        lock = {"schema": policy.SCHEMA, "legacy_roots": ["Sources"],
                "runtime_roots": list(policy.RUNTIME_ROOTS),
                "files": {p: policy.digest(root / p)
                          for p in ["Sources/Legacy.swift", "Package.swift"]}}
        (root / policy.LOCK).write_text(json.dumps(lock))
        return root



    def test_legacy_change_addition_removal_and_package_edit_fail(self):
        for mutation in ["change", "add", "remove", "package"]:
            with self.subTest(mutation=mutation):
                root = self.fixture()
                if mutation == "change":
                    (root / "Sources/Legacy.swift").write_text("// changed\n")
                elif mutation == "add":
                    (root / "Sources/NewHost.swift").write_text("// new\n")
                elif mutation == "remove":
                    (root / "Sources/Legacy.swift").unlink()
                else:
                    (root / "Package.swift").write_text("// new Swift target\n")
                self.assertFalse(policy.verify(root)["ok"])

    def test_non_rust_runtime_or_sidecar_host_fails(self):
        for path in ["rust-native/src/bridge.swift", "rust-native/src/bridge.m",
                     "rust/src/bridge.py", "rust-native/NewHost.swift",
                     "rust-native/web/runtime.js", "rust/NewHost.swift",
                     "rust-native/NewHost.rb", "rust/NewHost.go", "rust-native/NewHost.sh"]:
            with self.subTest(path=path):
                root = self.fixture()
                file = root / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text("// forbidden product code\n")
                self.assertFalse(policy.verify(root)["ok"])


    def test_symlink_and_missing_or_weakened_lock_fail_closed(self):
        root = self.fixture()
        file = root / "Sources/Legacy.swift"
        content = file.read_bytes()
        file.unlink()
        other = root / "old.swift"
        other.write_bytes(content)
        file.symlink_to(other)
        self.assertFalse(policy.verify(root)["ok"])
        root = self.fixture()
        (root / "rust-native/src/alias.rs").symlink_to(root / "rust/src/lib.rs")
        self.assertFalse(policy.verify(root)["ok"])
        root = self.fixture()
        external = root / "foreign"
        external.mkdir()
        (external / "Host.swift").write_text("// own symlink fixture")
        (root / "rust-native/host").symlink_to(external, target_is_directory=True)
        self.assertFalse(policy.verify(root)["ok"])
        root = self.fixture()
        lock = root / policy.LOCK
        lock.unlink()
        self.assertFalse(policy.verify(root)["ok"])
        root = self.fixture()
        lock = root / policy.LOCK
        value = json.loads(lock.read_text())
        value["files"].pop("Package.swift")
        lock.write_text(json.dumps(value))
        self.assertFalse(policy.verify(root)["ok"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
