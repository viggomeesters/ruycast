.PHONY: check build install source-tool export-source verify-source disk-status clean-build-caches
check:
	bash scripts/check.sh
build:
	bash scripts/build-native.sh -c release
install:
	cargo run --locked --offline --release --manifest-path rust-native/Cargo.toml --target-dir .build/installer-tools --bin ruycast-install -- install --repo "$(CURDIR)" --build
source-tool:
	cargo build --locked --manifest-path rust-native/Cargo.toml --bin ruycast-source
export-source:
	cargo run --quiet --locked --manifest-path rust-native/Cargo.toml --bin ruycast-source -- export --repo "$(CURDIR)" --out "$(OUT)"
verify-source:
	cargo run --quiet --locked --manifest-path rust-native/Cargo.toml --bin ruycast-source -- verify --source "$(OUT)"
disk-status:
	cargo run --quiet --locked --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- disk-status --repo "$(CURDIR)"
clean-build-caches:
	cargo run --quiet --locked --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- clean-build-caches --repo "$(CURDIR)"

.PHONY: verify-build version check-settings check-performance
verify-build:
	cargo run --quiet --locked --offline --target-dir "$(CURDIR)/.build/delivery-tools" --manifest-path "$(CURDIR)/rust-native/Cargo.toml" --bin ruycast-build -- compat build-receipt --repo "$(CURDIR)" -- verify
version:
	cargo run --quiet --locked --offline --manifest-path rust-native/Cargo.toml --target-dir .build/delivery-tools --bin ruycast-build -- compat version --repo "$(CURDIR)" --
check-settings:
	bash scripts/check-settings.sh
check-performance:
	bash scripts/check-performance.sh $(PERFORMANCE_ARGS)

.PHONY: install-preview
install-preview:
	CARGO_INCREMENTAL=0 cargo run --locked --release --manifest-path rust-native/Cargo.toml --bin ruycast-install -- install-preview "$(APP)"
