#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
source_fingerprint=$(python3 scripts/build_receipt.py fingerprint)
cargo build --manifest-path rust/Cargo.toml --target-dir .build/rust --release --locked
# SwiftPM does not track changes to an externally linked static archive.
# Invalidate Swift artifacts when the Rust archive changes, for both configurations.
archive_hash=$(shasum -a 256 .build/rust/release/librustcast_core.a | cut -d ' ' -f 1)
build_key="$archive_hash:$(pwd -P)"
previous_hash=$(cat .build/swift-rust.sha256 2>/dev/null || true)
if [[ "$build_key" != "$previous_hash" ]]; then
    swift package --scratch-path .build/swift clean
fi
swift build --scratch-path .build/swift "$@"
printf '%s\n' "$build_key" > .build/swift-rust.sha256

if [[ " $* " == *" -c release "* || " $* " == *" --configuration release "* || " $* " == *" --configuration=release "* ]]; then
    python3 scripts/build_receipt.py record --before "$source_fingerprint" > /dev/null
fi
