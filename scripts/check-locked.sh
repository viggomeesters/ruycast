#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
.build/swift/debug/LauncherCoreChecks
.build/swift/debug/RuycastApp --check-search
.build/swift/debug/RuycastApp --check-concurrency
python3 -m unittest discover -s Tests -p 'test_*.py' -v
git diff --check
