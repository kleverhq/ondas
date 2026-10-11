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

## Release metadata

`check_release.py` reads `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` from the
working directory. It requires matching stable versions and nonempty, dated
release notes, and prints those notes without modifying files. `--tag vX.Y.Z`
additionally checks the release tag. It uses only Python 3.11+'s standard library;
`test_check_release.py` covers metadata rejection and note extraction.

`./dev just release-check` also lists and verifies the crates.io package with
`cargo publish --dry-run`; it needs registry access but no publishing token.
