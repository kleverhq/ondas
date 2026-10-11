set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

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

# Check every FST/VCD in the pinned fixtures plus focused API regressions.
conformance: _inside
    cargo test --locked --profile conformance --test conformance -- --ignored --nocapture

# Check every FSDB in the pinned fixtures, plus focused regressions.
conformance-fsdb: _inside
    tools/repo/fsdb-sweep just _conformance-fsdb

_conformance-fsdb: _inside
    cargo test --locked --profile conformance --features fsdb-lib --test conformance fsdb_ -- --ignored --nocapture
    cargo test --locked --profile conformance --features fsdb-lib --lib fsdb_ -- --ignored --nocapture

# Execute VCD/FST Criterion workloads without timing thresholds.
bench-smoke: _inside
    cargo bench --locked --bench vcd -- --test
    cargo bench --locked --bench fst -- --test

# Execute vendor Criterion workloads against pinned fixtures.
bench-smoke-fsdb: _inside
    tools/repo/fsdb-sweep just _bench-smoke-fsdb

_bench-smoke-fsdb: _inside
    cargo bench --locked --features fsdb-lib --bench fsdb -- --test

# Measure FSDB workloads for each selected SDK; accepts Criterion arguments.
bench-fsdb *args: _inside
    tools/repo/fsdb-sweep just _bench-fsdb "$@"

_bench-fsdb *args: _inside
    cargo bench --locked --features fsdb-lib --bench fsdb -- "$@"

# Verify actual downstream linking, independent of Cargo's runtime environment.
fsdb-consumer: _inside
    python3 tools/repo/check_fsdb_consumer.py

# Validate each SDK in VERDI_HOMES, or the single VERDI_HOME.
ci-fsdb: _inside
    tools/repo/fsdb-sweep just _ci-fsdb

_ci-fsdb: _inside
    cargo clippy --locked --all-targets --features fsdb-lib -- -D warnings
    cargo test --locked --features fsdb-lib
    cargo +{{msrv}} check --locked --lib --features fsdb-lib
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --features fsdb-lib --no-deps
    just _conformance-fsdb
    just _bench-smoke-fsdb
    just fsdb-consumer
    RUSTUP_TOOLCHAIN={{msrv}} just fsdb-consumer

# Test repository automation without Docker or external fixtures.
tools-test: _inside
    python3 -B -m unittest discover -s tools/repo -p 'test_*.py'

# Run the same pre-commit checks against all tracked files.
pre-commit: _inside
    pre-commit run --all-files

# Install and verify waveform payloads from the initialized fixtures submodule.
fixtures-install: _inside
    @test -f fixtures/justfile || { echo "error: run git submodule update --init fixtures on the host first" >&2; exit 1; }
    just --justfile fixtures/justfile install

# Run offline quality checks without external fixtures (also used by pre-commit).
check-local: fmt-check lint check docs test tools-test

# Validate release metadata and the actual crates.io package without publishing.
release-check: _inside
    python3 -B tools/repo/check_release.py
    cargo package --list --locked
    cargo publish --dry-run --locked

# Run the full quality gate; fixtures must already be installed.
ci: check-local conformance bench-smoke release-check
