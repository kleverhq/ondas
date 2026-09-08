# Repository Hooks

`git-hook` is the reviewed source for the host `pre-commit` dispatcher. Install
it from the host with `./dev --install-hooks`; do not run the tracked source
directly. Installation copies the dispatcher and launcher into the current
worktree's Git directory and sets worktree-local `core.hooksPath`. Reinstall to
activate reviewed source changes.

The hook maps the worktree, Git directories, and exact commit index into an
already running devcontainer, then invokes container Pre-commit through the
installed launcher's `--exec-only` mode. It never starts or rebuilds a container.
`.pre-commit-config.yaml` selects checks through `just`; Pre-commit isolates staged
changes from unstaged edits. A failed check blocks the commit.

Host dependencies are Git, Bash, standard Linux utilities, Docker, and the Dev
Container CLI. Installing hooks needs neither a running container nor host Python
or Pre-commit. The container supplies Pre-commit and the project toolchain.

Run the self-contained hook tests with `./dev just tools-test`, or all tracked-file
checks with `./dev just pre-commit`. Tests use temporary Git repositories and fake
container commands; they do not change the caller's Git configuration or manage
real containers. Runtime ownership and contributor rules are documented in
[automation](../../docs/automation.md).
