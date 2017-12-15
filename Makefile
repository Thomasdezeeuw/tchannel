ifdef CI
TEST_FLAGS = "--all --verbose"
else
TEST_FLAGS = "--all"
endif

version:
	cargo --version
	rustc --version

build:
	cargo build
	cargo build --all-features

test: version build
	cargo test $(TEST_FLAGS) --no-default-features
	cargo test $(TEST_FLAGS) --all-features
	# Make sure release mode works as well.
	cargo test $(TEST_FLAGS) --release --no-default-features
	cargo test $(TEST_FLAGS) --release --all-features

stress_test:
	cargo test $(TEST_FLAGS) --no-default-features -- --ignored
	cargo test $(TEST_FLAGS) --all-features -- --ignored
	# Make sure release mode works as well.
	cargo test $(TEST_FLAGS) --release --no-default-features -- --ignored
	cargo test $(TEST_FLAGS) --release --all-features -- --ignored

.PHONY: version build test stress_test
