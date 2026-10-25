#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/local_signing.py prepare >/dev/null
bash scripts/build-native.sh -c release
python3 scripts/install.py
