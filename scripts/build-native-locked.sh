#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Compatibility name. Rust owns the one inherited artifact lease.
exec python3 scripts/build_receipt.py build "$@"
