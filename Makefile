CARGO_BUILD_JOBS ?= 4
export CARGO_BUILD_JOBS

TURRIS_TARGET := armv7-unknown-linux-musleabihf

.PHONY: build release run test test-unit test-integration lint fmt check clean cross

build:
	cargo build

release:
	cargo build --release

run:
	cargo run --bin healthcheck

test:
	cargo test

test-unit:
	cargo test --lib --bins

test-integration:
	cargo test --test '*'

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt

check: fmt lint test

clean:
	cargo clean

# Cross-compile for Turris Omnia/MOX (requires Docker + `cross`)
cross:
	cross build --release --target $(TURRIS_TARGET)
