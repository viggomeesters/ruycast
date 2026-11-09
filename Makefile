.PHONY: check build install source-tool export-source verify-source
check:
	bash scripts/check.sh
build:
	bash scripts/build-native.sh -c release
install:
	bash scripts/install.sh
source-tool:
	cargo build --locked --manifest-path rust-native/Cargo.toml --bin ruycast-source
export-source:
	cargo run --quiet --locked --manifest-path rust-native/Cargo.toml --bin ruycast-source -- export --repo "$(CURDIR)" --out "$(OUT)"
verify-source:
	cargo run --quiet --locked --manifest-path rust-native/Cargo.toml --bin ruycast-source -- verify --source "$(OUT)"

.PHONY: verify-build version check-settings check-performance
verify-build:
	python3 scripts/build_receipt.py verify
version:
	python3 scripts/product_version.py
check-settings:
	bash scripts/check-settings.sh
check-performance:
	bash scripts/check-performance.sh $(PERFORMANCE_ARGS)
