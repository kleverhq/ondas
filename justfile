set shell := ["bash", "-euo", "pipefail", "-c"]

msrv := `sed -n 's/^rust-version = "\([^"]*\)"/\1/p' Cargo.toml`

# Show the public command surface.
default:
    @just --list

_inside:
    @test "${ONDAS_IN_CONTAINER:-}" = 1 || { echo "error: run this recipe through ./dev" >&2; exit 1; }

# Format Rust sources.
fmt: _inside
    cargo fmt --all

# Check Rust formatting without modifying files.
fmt-check: _inside
    cargo fmt --all -- --check

# Run Clippy on the library and test targets.
lint: _inside
    cargo clippy --locked --all-targets -- -D warnings

# Compile the library and test targets on the pinned development toolchain.
check: _inside
    cargo check --locked --all-targets

# Generate the docs.rs-equivalent API documentation.
docs: _inside
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --no-deps

# Check the public library on the Cargo.toml rust-version.
msrv: _inside
    cargo +{{msrv}} check --locked --lib

# Run self-contained Rust tests and public documentation examples.
test: _inside
    cargo test --locked

# Check every FST/VCD in the locked provider plus focused API regressions.
conformance: _inside
    cargo test --locked --test conformance -- --ignored --nocapture

# Check every public FSDB and available private FSDB, plus focused regressions.
# Set ONDAS_REQUIRE_PRIVATE_FIXTURES=1 to require the complete private selection.
conformance-fsdb: _inside
    cargo test --locked --features fsdb-lib --test conformance fsdb_ -- --ignored --nocapture

# Verify actual downstream linking, independent of Cargo's runtime environment.
fsdb-consumer: _inside
    python3 tools/repo/check_fsdb_consumer.py

# Validate the optional backend; VERDI_HOME must select an installed SDK.
ci-fsdb: _inside
    cargo clippy --locked --all-targets --features fsdb-lib -- -D warnings
    cargo test --locked --features fsdb-lib
    cargo +{{msrv}} check --locked --lib --features fsdb-lib
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --features fsdb-lib --no-deps
    just conformance-fsdb
    just fsdb-consumer
    RUSTUP_TOOLCHAIN={{msrv}} just fsdb-consumer

# Test repository automation without Docker or external fixtures.
tools-test: _inside
    python3 -B -m unittest discover -s tools/repo -p 'test_*.py'

# Run the same pre-commit checks against all tracked files.
pre-commit: _inside
    pre-commit run --all-files

# Install and verify the tagged public fixture provider from GitHub.
fixtures-install: _inside
    python3 -B tools/repo/install_fixtures.py

# Run offline quality checks without external fixtures (also used by pre-commit).
check-local: fmt-check lint check docs test tools-test

# Run the full quality gate; fixtures must already be installed.
ci: check-local conformance
