# Repository Automation

`git-hook` is the reviewed source for the host `pre-commit` dispatcher. Install
it from the host with `./dev --install-hooks`; do not run the tracked source
directly. Installation copies the dispatcher and launcher into the current
worktree's Git directory and sets worktree-local `core.hooksPath`. Reinstall to
activate reviewed source changes.

The hook maps the worktree, Git directories, and exact commit index into an
already running devcontainer, then invokes container Pre-commit through the
installed launcher's `--exec-only` mode. It never starts or rebuilds a container.
`.pre-commit-config.yaml` selects checks through `just`; Pre-commit isolates staged
changes from unstaged edits. It runs fixture-free `just check-local`; a failed check
blocks the commit. Full `just ci` additionally runs conformance.

Host dependencies are Git, Bash, standard Linux utilities, Docker, and the Dev
Container CLI. Installing hooks needs neither a running container nor host Python
or Pre-commit. The container supplies Pre-commit and the project toolchain.

Run the self-contained hook tests with `./dev just tools-test`, or all tracked-file
checks with `./dev just pre-commit`. Tests use temporary Git repositories and fake
container commands; they do not change the caller's Git configuration or manage
real containers. Runtime ownership and contributor rules are documented in
[automation](../../docs/automation.md).

## Fixture installation

`./dev just fixtures-install` runs `install_fixtures.py` inside the container.
It requires Git, Python 3.11+, network access and `ONDAS_FIXTURES`. The script reads
`fixtures.lock.toml`, clones the public provider's exact version tag into that root,
checks the checkout/catalog, and runs its `install.py`. It modifies only the chosen
provider directory; existing mismatched or dirty checkouts are rejected rather than
reset. The upstream installer downloads release assets and verifies size/SHA-256,
including cached payloads. `GITHUB_TOKEN` is optional for authenticated API access.
No missing-asset bypass is used. `test_install_fixtures.py` exercises clone/reuse,
revision/catalog rejection and installer execution without network access.
