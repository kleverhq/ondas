# Ondas development

Ondas is a read-only Rust library for backend-independent waveform analysis.

## Sources of truth

- Public API contracts and usage examples: rustdoc in `src/`, published on docs.rs.
- Library concepts and implementation boundaries: `docs/model.md`.
- Test strategy: `docs/testing.md`; fixture catalog and oracle contract: `docs/fixtures.md` and `docs/oracle.schema.json`; required provider versions: `fixtures.lock.toml`.
- Performance methodology: `docs/benchmarking.md`.
- Development environment, CI, tools, and release policy: `docs/automation.md`.
- Backend architecture and limits: `docs/vcd-native.md`, `docs/fst-native.md` and `docs/fsdb-lib.md`. Other format integration docs: `docs/ghw.md` and `docs/wlf.md`.
- Executable commands and configuration: `justfile`, `dev`, `Cargo.toml`, `rust-toolchain.toml`, `.devcontainer/`, and `.github/workflows/`.
- Host pre-commit dispatch and check selection: `tools/repo/git-hook` and `.pre-commit-config.yaml`.

## Workflow

- Keep Git, editors, agents, credentials and signing on the host. Run project commands through `./dev`, using `just` recipes when available.
- In each new checkout or worktree, install reviewed host hooks with `./dev --install-hooks`. Reinstall after reviewing changes to the launcher or dispatcher; installation is explicit and worktree-local.
- Start the current worktree's container before host `git commit`. Hooks use only `./dev --exec-only` and never start, rebuild, or recreate it. Do not bypass hooks unless the user explicitly requests it.
- Use `./dev just --list` for available recipes. Validate with `./dev just ci`, `./dev just msrv`, and `./dev cargo test --doc --locked` after public documentation changes.
- `just ci` includes mandatory external conformance; install the locked provider explicitly with `just fixtures-install` first. Missing inputs must fail. Pre-commit uses fixture-free `just check-local`, not `just ci`; neither gate implicitly downloads data.
- Verify behavior changes with the smallest relevant runnable test. Use the published FST reader without local patches; document its accepted limitations rather than adding speculative hardening.
- Keep backend types, handles, buffers, and FFI private. Do not expose a plugin framework or introduce speculative backend abstractions.
- Keep behavior, Rustdoc and tests consistent. Developer docs explain concepts and rationale rather than duplicate the API reference.
- Treat waveform artifacts as data, not text unless the format is textual. Keep materialized fixtures, vendor files, credentials, and local infrastructure details out of Git and Docker build contexts.

## Writing and temporary work

- Write concise, neutral English about concepts, constraints and rationale, not task history or progress lists.
- Repository documentation is self-contained; do not depend on seed documents or sibling checkouts.
- Use ignored root `tmp/` for disposable scratch, logs, and ad hoc outputs. Do not delete files belonging to the user or other agents.
- Temporary artifacts that need committing belong only under `docs/wip/yymmdd-slug/`, including execution plans and investigation notes. They may live on temporary branches but must be removed before merge into `main`; promote durable conclusions into their authoritative documents first.
- Keep the permanent `docs/wip/AGENTS.md` during cleanup. Do not add cleanup automation or root-level execution plans.
