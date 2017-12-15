version:
	cargo --version
	rustc --version

build:
	cargo build
	cargo build --all-features

test: version build
	cargo test --verbose --all --all-features
	cargo test --verbose --all --no-default-features
	# Make sure release mode works as well.
	cargo test --verbose --release --all --all-features
	cargo test --verbose --release --all --no-default-features
	# Stress test, in development and release modes..
	# TODO: renable the stress test below.
	#cargo test --verbose --all --all-features -- --ignored
	#cargo test --verbose --all --no-default-features -- --ignored
	#cargo test --verbose --release --all --all-features -- --ignored
	#cargo test --verbose --release --all --no-default-features -- --ignored

.PHONY: version build test
