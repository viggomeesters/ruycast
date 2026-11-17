#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Compatibility name. Rust owns the one inherited artifact lease.
root="$(pwd -P)"
exec cargo run --quiet --locked --offline --target-dir "$root/.build/delivery-tools" --manifest-path "$root/rust-native/Cargo.toml" --bin ruycast-build -- compat build-receipt --repo "$root" -- build "$@"
