# Repository tools

## Hooks

Install the reviewed `git-hook` dispatcher with `./dev --install-hooks` on the
host; do not run its tracked source directly. Installation copies it and `dev`
into the worktree's Git directory and sets local `core.hooksPath`. Reinstall
after reviewing changes.

The hook maps the worktree, Git directories and exact commit index into an already
running container, then calls Pre-commit through `--exec-only`. It never starts
or rebuilds a container. Pre-commit isolates staged changes and runs the
fixture-free `just check-local` from `.pre-commit-config.yaml`. Failure blocks
the commit. Full `just ci` also runs conformance.

The host needs Git, Bash, standard Linux utilities, Docker and the Dev Container
CLI. Hook installation needs no running container, host Python or host Pre-commit;
the container supplies the toolchain.

`./dev just tools-test` tests hooks with temporary Git repositories and fake
container commands, without changing the caller's Git configuration or managing
real containers. `./dev just pre-commit` checks all tracked files. See
[automation](../../docs/automation.md) for runtime and contributor rules.

## Fixture installation

Initialize the pinned `fixtures/` submodule on the host with
`git submodule update --init fixtures`. `./dev just fixtures-install` then invokes
its `just install` through a thin Just recipe; an uninitialized checkout fails
with setup instructions. No Ondas-specific installer is needed.

The fixture installer uses Python, Just and network access to download release
assets and verify sizes and SHA-256, including cached payloads. `GITHUB_TOKEN` is
optional for authenticated API access. Metadata and the unified schema come from
the pinned submodule; waveforms remain ignored data. Neither tests nor quality
gates install them implicitly. See [fixture integration](../../docs/fixtures.md)
for revision updates and consumer checks.

## FSDB SDK sweep

`fsdb-sweep` runs its command once per path in the whitespace-separated
`VERDI_HOMES` environment variable. An empty or unset list uses `VERDI_HOME`.
List paths must have no whitespace. It sets the selected single path, clears
the list in child commands, and puts each build under
`${CARGO_TARGET_DIR:-target}/fsdb-sdk/<position>`. It prints per-SDK outcomes,
continues after failures and returns a failed aggregate status. Cancellation
stops the sweep.

Use `./dev just ci-fsdb`, `conformance-fsdb`, `bench-smoke-fsdb` or `bench-fsdb`.
The wrapper needs Bash and `env`; recipes supply Cargo and Just. It modifies
only the command's build and benchmark output. Configure actual installation
paths and mounts in ignored local files. See [automation](../../docs/automation.md).
`test_fsdb_sweep.py` checks selection, output isolation, argument preservation,
failure aggregation and cancellation with fake commands through `just tools-test`.

`check_fsdb_consumer.py` builds a temporary downstream crate under ignored `tmp/`
and launches it without Cargo's loader environment. A confirmed newer-file error
prints a skip after process startup. All other failures remain failures.

## SHM SDK sweep

`shm-sweep` follows the FSDB sweep rules with `XCELIUM_HOMES`, `XCELIUM_HOME`,
and `${CARGO_TARGET_DIR:-target}/shm-sdk/<position>`. It requires the selected
`tools/lib/64bit/libsst2.so`. Each child gets a runtime directory with a symlink
to that reader at the front of `LD_LIBRARY_PATH`. Existing loader paths follow
it. The wrapper needs Bash, `env`, `mkdir`, `ln`, and `realpath`. It modifies only
build output and its runtime symlink.

Use `./dev just ci-shm`, `conformance-shm`, `bench-smoke-shm` or `bench-shm`.
The recipes select locally supplied paths without a fixed version list. Each
reader uses the same quoted-value transport.
`test_shm_sweep.py` checks selection, argument preservation, build and loader
isolation, missing readers, failure aggregation and cancellation with fake commands.
`check_shm_consumer.py` verifies a separately built application's selected native
dependency and launches it without Cargo's runtime environment.

## WLF SDK sweep

`wlf-sweep` applies the same rules as `fsdb-sweep`, using `QUESTA_HOMES` and
`QUESTA_HOME`. It puts each build under
`${CARGO_TARGET_DIR:-target}/wlf-sdk/<position>`. Use `./dev just ci-wlf`,
`conformance-wlf`, `bench-build-wlf`, `bench-smoke-wlf` or `bench-wlf`.
`ci-wlf` includes Criterion `--test` smoke, without performance measurements.
`test_wlf_sweep.py` checks the same command behavior with fake SDK paths and
commands through `just tools-test`.

## Release metadata

`check_release.py` reads `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` from the
working directory. It requires matching stable versions and nonempty, dated
release notes, and prints those notes without modifying files. `--tag vX.Y.Z`
additionally checks the release tag. It uses only Python 3.11+'s standard library;
`test_check_release.py` covers metadata rejection and note extraction.

`./dev just release-check` also lists and verifies the crates.io package with
`cargo publish --dry-run`; it needs registry access but no publishing token.

## Vendor consumers

`check_fsdb_consumer.py` and `check_wlf_consumer.py` build a separate crate and
launch its executable without Cargo's loader environment. They need the selected
SDK and an installed fixture. The WLF check also verifies the propagated absolute
vendor dependency and rejects a separate Ondas shim dependency. Run their Just
recipes through the selected ignored SDK profile on development Rust and MSRV.
