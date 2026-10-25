.PHONY: check build install
check:
	bash scripts/check.sh
build:
	bash scripts/build-native.sh -c release
install:
	bash scripts/install.sh
