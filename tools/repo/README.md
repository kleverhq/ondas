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

`./dev just fixtures-install` runs `install_fixtures.py` in the container. It needs
Git, Python 3.11+, network access and `ONDAS_FIXTURES`. It reads the lock, clones
the provider's exact tag, checks checkout/catalog identity and runs its
`install.py`. Only the selected provider directory is modified. Mismatched or
dirty checkouts fail rather than being reset.

The provider installer downloads release assets and verifies sizes and SHA-256,
including cached payloads. It does not bypass missing assets. `GITHUB_TOKEN` is
optional for authenticated API access. `test_install_fixtures.py` checks clone,
reuse, revision/catalog rejection and installer execution without network access.
