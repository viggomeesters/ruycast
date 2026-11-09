#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
exec bash scripts/check.sh "$@"

# Historical oracle body; execution uses the maintained Rust pipeline above.
.build/swift/debug/LauncherCoreChecks
.build/swift/debug/RuycastApp --check-search
.build/swift/debug/RuycastApp --check-concurrency
python3 -m unittest discover -s Tests -p 'test_*.py' -v
git diff --check
