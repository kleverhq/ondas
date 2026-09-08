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
    cargo clippy --locked --all-targets --all-features -- -D warnings

# Compile the library and test targets on the pinned development toolchain.
check: _inside
    cargo check --locked --all-targets --all-features

# Generate the docs.rs-equivalent API documentation.
docs: _inside
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --all-features --no-deps

# Check the public library on the Cargo.toml rust-version.
msrv: _inside
    cargo +{{msrv}} check --locked --lib --all-features

# Run self-contained Rust tests and public documentation examples.
test: _inside
    cargo test --locked --all-features

# Check every FST in the locked provider plus focused API regressions.
conformance: _inside
    cargo test --locked --test fst_conformance -- --ignored --nocapture

# Test repository automation without Docker or external fixtures.
tools-test: _inside
    python3 -B -m unittest discover -s tools/repo -p 'test_*.py'

# Run the same pre-commit checks against all tracked files.
pre-commit: _inside
    pre-commit run --all-files

# Run public static-quality, self-contained Rust, and repository-tool checks.
ci: fmt-check lint check docs test tools-test
