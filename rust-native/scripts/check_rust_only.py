#!/usr/bin/env python3
"""Reject new legacy/product-language drift during the full Rust migration.

This is a read-only local gate, not a lock generator or a production-readiness
claim. The actual worktree at the explicit policy boundary is the baseline.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat

SCHEMA = "ruycast.rust-only-reference-lock.v1"
LEGACY_ROOT = "Sources"
PACKAGE = "Package.swift"
RUNTIME_ROOTS = ("rust/src", "rust-native/src")
LOCK = "rust-native/swift-reference-lock.json"
PRODUCT_ROOTS = ("rust", "rust-native")
METADATA_NAMES = {"Cargo.toml", "Cargo.lock", "README.md", "LICENSE", "LICENSE.md",
                  "AGENTS.md", ".gitignore", "swift-reference-lock.json"}
RESOURCE_SUFFIXES = {".json", ".toml", ".plist", ".png", ".svg", ".icns", ".md", ".txt"}


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def regular(path):
    return stat.S_ISREG(path.lstat().st_mode)


def verify(root):
    root = Path(root)
    errors = []
    try:
        lock_path = root / LOCK
        if not regular(lock_path):
            raise ValueError("reference lock must be a regular file")
        lock = json.loads(lock_path.read_text())
        expected = lock["files"]
        if (lock["schema"] != SCHEMA or lock["legacy_roots"] != [LEGACY_ROOT]
                or lock["runtime_roots"] != list(RUNTIME_ROOTS)
                or not isinstance(expected, dict) or PACKAGE not in expected):
            raise ValueError("invalid reference-lock contract")
        for path, value in expected.items():
            p = Path(path)
            if (p.is_absolute() or ".." in p.parts
                    or (path != PACKAGE and not path.startswith(LEGACY_ROOT + "/"))
                    or not isinstance(value, str) or len(value) != 64
                    or any(c not in "0123456789abcdef" for c in value)):
                raise ValueError("invalid frozen-file entry")
        actual = {}
        legacy = root / LEGACY_ROOT
        if legacy.is_symlink() or not legacy.is_dir():
            raise ValueError("legacy source root must be a real directory")
        for path in sorted(legacy.rglob("*")):
            relative = path.relative_to(root).as_posix()
            if path.is_symlink():
                errors.append("legacy symlink: " + relative)
            elif path.is_file():
                if not regular(path):
                    errors.append("nonregular legacy input: " + relative)
                else:
                    actual[relative] = digest(path)
            elif not path.is_dir():
                errors.append("nonregular legacy input: " + relative)
        package = root / PACKAGE
        if not regular(package):
            raise ValueError("Package.swift must be a regular file")
        actual[PACKAGE] = digest(package)
        for path in sorted(expected.keys() | actual.keys()):
            if path not in expected:
                errors.append("added legacy input: " + path)
            elif path not in actual:
                errors.append("removed legacy input: " + path)
            elif actual[path] != expected[path]:
                errors.append("changed legacy input: " + path)
        runtime_count = 0
        for name in RUNTIME_ROOTS:
            folder = root / name
            if folder.is_symlink() or not folder.is_dir():
                errors.append("runtime source root must be a real directory: " + name)
                continue
            for path in sorted(folder.rglob("*")):
                relative = path.relative_to(root).as_posix()
                if path.is_symlink():
                    errors.append("runtime source symlink: " + relative)
                elif path.is_file():
                    if path.name == ".DS_Store":
                        continue
                    if not regular(path) or path.suffix != ".rs":
                        errors.append("non-Rust runtime source: " + relative)
                    else:
                        runtime_count += 1
                elif not path.is_dir():
                    errors.append("nonregular runtime source: " + relative)
        # Fail closed on unknown product files in either crate, rather than
        # enumerating a few forbidden languages. Tooling/caches are explicit.
        for product in PRODUCT_ROOTS:
            product_root = root / product
            if product_root.is_symlink() or not product_root.is_dir():
                errors.append("product root must be a real directory: " + product)
                continue
            for parent, directories, filenames in os.walk(product_root, followlinks=False):
                folder = Path(parent)
                if folder == product_root:
                    for name in ("tests", "scripts"):
                        if (folder / name).is_symlink():
                            errors.append("tooling source symlink: " + product + "/" + name)
                    directories[:] = [d for d in directories
                                      if d not in {"target", "tests", "scripts", "src", ".git"}]
                for name in list(directories):
                    if (folder / name).is_symlink():
                        errors.append("product directory symlink: " + (folder / name).relative_to(root).as_posix())
                        directories.remove(name)
                for name in filenames:
                    path = folder / name
                    relative = path.relative_to(product_root)
                    if name == ".DS_Store":
                        continue
                    resource = (relative.parts[0] in {"assets", "resources", "docs", ".cargo"}
                                and path.suffix.lower() in RESOURCE_SUFFIXES)
                    if (path.is_symlink() or not regular(path)
                            or not (path.suffix == ".rs" or name in METADATA_NAMES or resource)):
                        errors.append("non-Rust/unknown product input: " + path.relative_to(root).as_posix())
        return {"ok": not errors, "policy": "Rust-only new product code",
                "frozenLegacyFiles": len(expected), "rustRuntimeFiles": runtime_count,
                "lockSHA256": digest(lock_path), "errors": errors,
                "productionReady": False}
    except (OSError, ValueError, KeyError, TypeError) as error:
        return {"ok": False, "errors": ["Rust-only policy: " + str(error)],
                "productionReady": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path,
                        default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    result = verify(args.root)
    print(json.dumps(result, indent=2))
    return 0 if result["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
