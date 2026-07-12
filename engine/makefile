# =============================================================================
# Crypto Exchange Matching Engine — Makefile
# =============================================================================
#
# Usage:
#   make run                        # No WAL, with tracing (default)
#   make run-wal                    # With WAL, with tracing
#   make run-no-trace               # No WAL, no tracing
#   make run-wal-no-trace           # With WAL, no tracing
#   make run-debug                  # No WAL, debug-level tracing
#   make run-trace                  # No WAL, trace-level tracing (ultra verbose)
#
#   make run LOG=trace              # Override RUST_LOG level
#   make run-wal LOG=debug,hyper=trace
#
#   make build                      # Compile
#   make test                       # Run all tests
#   make test-wal                   # Run WAL tests only
#   make test-engine                # Run engine tests only
#   make release                    # Optimized build
#   make clean                      # Clean artifacts
#   make doc                        # Generate docs
#   make lint                       # Clippy warnings
#
# =============================================================================

# RUST_LOG override — defaults to info,engine=debug
LOG ?= info,engine=debug

# WAL path
WAL_PATH ?= engine.wal

# --- Run targets ------------------------------------------------------------

.PHONY: run run-wal run-no-trace run-wal-no-trace run-debug run-trace

run: build
	WAL_ENABLED=false \
	RUST_LOG=$(LOG) \
	cargo run

run-wal: build
	WAL_ENABLED=true \
	WAL_PATH=$(WAL_PATH) \
	RUST_LOG=$(LOG) \
	cargo run

run-no-trace: build
	WAL_ENABLED=false \
	TRACING_ENABLED=false \
	cargo run

run-wal-no-trace: build
	WAL_ENABLED=true \
	WAL_PATH=$(WAL_PATH) \
	TRACING_ENABLED=false \
	cargo run

run-debug: build
	WAL_ENABLED=false \
	RUST_LOG=debug,engine=trace \
	cargo run

run-trace: build
	WAL_ENABLED=false \
	RUST_LOG=trace \
	cargo run

# --- Build targets -----------------------------------------------------------

.PHONY: build release clean

build:
	cargo build

release:
	cargo build --release

clean:
	cargo clean

# --- Test targets ------------------------------------------------------------

.PHONY: test test-engine test-wal test-snowflake

test:
	cargo test

test-engine:
	cargo test --test engine_test

test-wal:
	cargo test --test wal_test

test-snowflake:
	cargo test --test snowflake_test

# --- Quality targets ---------------------------------------------------------

.PHONY: doc doc-open lint fmt

doc:
	cargo doc

doc-open:
	cargo doc --open

lint:
	cargo clippy -- -D warnings

fmt:
	cargo fmt --check

# --- Dev helpers -------------------------------------------------------------

.PHONY: clean-wal clean-logs

clean-wal:
	rm -f $(WAL_PATH)

clean-logs:
	rm -rf logs/
