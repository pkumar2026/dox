.PHONY: check fmt clippy test test-ignored run install build release

check: fmt-check clippy test

fmt:
	cargo fmt

fmt-check:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

test:
	cargo test

test-ignored:
	cargo test -- --ignored

run:
	cargo run

build:
	cargo build

release:
	cargo build --release

install:
	cargo install --path .
