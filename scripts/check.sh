#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo test --manifest-path rust/Cargo.toml --target-dir .build/rust --locked
bash scripts/build-native.sh
swift run --scratch-path .build/swift LauncherCoreChecks
python3 -m unittest discover -s Tests -p 'test_*.py' -v
git diff --check
