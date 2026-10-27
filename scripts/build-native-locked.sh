#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/prepare_release.py --recover
source_fingerprint=$(python3 scripts/build_receipt.py fingerprint)
# Keep the shared kernel speed-optimized; size-optimize only the CLI adapter.
cargo build --manifest-path rust/Cargo.toml --target-dir .build/rust --release --locked --lib
cargo rustc --manifest-path rust/Cargo.toml --target-dir .build/rust --release --locked --bin ruycast -- -C opt-level=s
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
    python3 scripts/prepare_release.py
    python3 scripts/build_receipt.py record --before "$source_fingerprint" > /dev/null
fi
