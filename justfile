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

# Run Clippy on the public library.
lint: _inside
    cargo clippy --locked --lib --all-features -- -D warnings

# Check the public library on the pinned stable toolchain.
check: _inside
    cargo check --locked --lib --all-features

# Generate the docs.rs-equivalent API documentation.
docs: _inside
    RUSTDOCFLAGS="-D warnings" cargo doc --locked --lib --all-features --no-deps

# Check the public library on the Cargo.toml rust-version.
msrv: _inside
    cargo +{{msrv}} check --locked --lib --all-features

# Test repository automation without Docker or external fixtures.
tools-test: _inside
    python3 -B -m unittest discover -s tools/repo -p 'test_*.py'

# Run the same pre-commit checks against all tracked files.
pre-commit: _inside
    pre-commit run --all-files

# Run public static-quality and repository-tool checks.
ci: fmt-check lint check docs tools-test
