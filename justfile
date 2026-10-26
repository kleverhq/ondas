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

# Check built-in readers against the pinned fixtures plus focused API regressions.
conformance: _inside
    cargo test --locked --profile conformance --test conformance -- --ignored --nocapture
    cargo test --locked --profile conformance --test exact_names -- --ignored

# Check every FSDB in the pinned fixtures, plus focused regressions.
conformance-fsdb: _inside
    tools/repo/fsdb-sweep just _conformance-fsdb

_conformance-fsdb: _inside
    cargo test --locked --profile conformance --features fsdb-lib --test conformance fsdb_ -- --ignored --nocapture
    cargo test --locked --profile conformance --features fsdb-lib --lib fsdb_ -- --ignored --nocapture
    cargo test --locked --profile conformance --features fsdb-lib --test exact_names fsdb_ -- --ignored

# Check every SHM directory and focused native-reader regressions.
conformance-shm: _inside
    tools/repo/shm-sweep just _conformance-shm

_conformance-shm: _inside
    cargo test --locked --profile conformance --features shm-lib --test conformance shm_ -- --ignored --nocapture
    cargo test --locked --profile conformance --features shm-lib --lib shm_ -- --ignored --nocapture
    cargo test --locked --profile conformance --features shm-lib --test exact_names shm_ -- --ignored

# Launch a SHM application without Cargo's runtime environment.
shm-consumer: _inside
    python3 tools/repo/check_shm_consumer.py


# Check every WLF in the pinned fixtures, plus directed SDK regressions.
conformance-wlf: _inside
    tools/repo/wlf-sweep just _conformance-wlf

_conformance-wlf: _inside
    cargo test --locked --profile conformance --features wlf-lib --test conformance wlf_ -- --ignored --nocapture
    cargo test --locked --profile conformance --features wlf-lib --test wlf_lib -- --ignored
    cargo test --locked --profile conformance --features wlf-lib --lib wlf_ -- --ignored
    cargo test --locked --profile conformance --features wlf-lib --test exact_names wlf_ -- --ignored

# Execute built-in Criterion workloads without timing thresholds.
bench-smoke: _inside
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --bench vcd -- --test
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --bench fst -- --test
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --bench ghw -- --test
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --bench wdb -- --test

# Execute vendor Criterion workloads against pinned fixtures.
bench-smoke-fsdb: _inside
    tools/repo/fsdb-sweep just _bench-smoke-fsdb

_bench-smoke-fsdb: _inside
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --features fsdb-lib --bench fsdb -- --test
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --features fsdb-lib --bench wdb -- --test

# Measure FSDB workloads for each selected SDK; accepts Criterion arguments.
bench-fsdb *args: _inside
    tools/repo/fsdb-sweep just _bench-fsdb "$@"

_bench-fsdb *args: _inside
    cargo bench --locked --features fsdb-lib --bench fsdb -- "$@"

# Execute SHM workloads for each selected SDK without measurements.
bench-smoke-shm: _inside
    tools/repo/shm-sweep just _bench-smoke-shm

_bench-smoke-shm: _inside
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --features shm-lib --bench shm -- --test

# Measure SHM workloads for each selected SDK; accepts Criterion arguments.
bench-shm *args: _inside
    tools/repo/shm-sweep just _bench-shm "$@"

_bench-shm *args: _inside
    cargo bench --locked --features shm-lib --bench shm -- "$@"

# Validate each SDK in XCELIUM_HOMES, or the single XCELIUM_HOME.
ci-shm: _inside
    tools/repo/shm-sweep just _ci-shm

_ci-shm: _inside
    cargo clippy --locked --all-targets --features shm-lib -- -D warnings
    cargo test --locked --features shm-lib
    cargo +{{msrv}} check --locked --lib --features shm-lib
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --features shm-lib --no-deps
    just _conformance-shm
    just _bench-smoke-shm
    just shm-consumer
    RUSTUP_TOOLCHAIN={{msrv}} just shm-consumer

# Compile WLF workloads without executing Criterion or opening fixtures.
bench-build-wlf: _inside
    tools/repo/wlf-sweep just _bench-build-wlf

_bench-build-wlf: _inside
    cargo bench --locked --features wlf-lib --bench wlf --no-run

# Execute WLF Criterion workloads without timing thresholds.
bench-smoke-wlf: _inside
    tools/repo/wlf-sweep just _bench-smoke-wlf

_bench-smoke-wlf: _inside
    CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-target}" cargo bench --locked --features wlf-lib --bench wlf -- --test

# Measure WLF workloads for each selected SDK; accepts Criterion arguments.
bench-wlf *args: _inside
    tools/repo/wlf-sweep just _bench-wlf "$@"

_bench-wlf *args: _inside
    cargo bench --locked --features wlf-lib --bench wlf -- "$@"

# Verify actual downstream linking, independent of Cargo's runtime environment.
fsdb-consumer: _inside
    python3 tools/repo/check_fsdb_consumer.py

# Verify WLF linking in a separate downstream executable.
wlf-consumer: _inside
    python3 tools/repo/check_wlf_consumer.py

# Validate each SDK in QUESTA_HOMES, or the single QUESTA_HOME.
ci-wlf: _inside
    tools/repo/wlf-sweep just _ci-wlf

_ci-wlf: _inside
    cargo clippy --locked --all-targets --features wlf-lib -- -D warnings
    cargo test --locked --features wlf-lib
    cargo +{{msrv}} check --locked --lib --features wlf-lib
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --features wlf-lib --no-deps
    just _conformance-wlf
    just _bench-smoke-wlf
    just wlf-consumer
    RUSTUP_TOOLCHAIN={{msrv}} just wlf-consumer

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
