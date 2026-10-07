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

## Release metadata

`check_release.py` reads `Cargo.toml`, `Cargo.lock` and `CHANGELOG.md` from the
working directory. It requires matching stable versions and nonempty, dated
release notes, and prints those notes without modifying files. `--tag vX.Y.Z`
additionally checks the release tag. It uses only Python 3.11+'s standard library;
`test_check_release.py` covers metadata rejection and note extraction.

`./dev just release-check` also lists and verifies the crates.io package with
`cargo publish --dry-run`; it needs registry access but no publishing token.
