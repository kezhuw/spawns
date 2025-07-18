all: check build test-matrix

check: check_fmt lint doc

verify: check build test

fmt:
	cargo +nightly fmt --all

check_fmt:
	cargo +nightly fmt --all -- --check

lint:
	cargo clippy --all-targets --all-features --no-deps -- -D clippy::all

build:
	cargo build-all-features

test:
	cargo test --all-features

test-matrix:
	cargo fc --fail-fast test

doc:
	cargo doc --all-features
