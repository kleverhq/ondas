# Worktrees, native environments and release boundaries

`dev`, `justfile`, `.devcontainer/` and `.github/workflows/ci.yml` own executable setup and checks. `AGENTS.md` gives the required contributor commands. Public checks use the nonvendor container; optional SDK features must not make the public gate depend on local installations.

## Configuration is executable and worktree-local

The host sources `.env` as Bash. It is executable configuration, not a passive dotenv parser. `ONDAS_DEV_CONFIG` selects a profile inside the worktree. Forward only the environment and mounts that profile needs; keep private infrastructure outside the public image and Docker context.

The launcher fingerprints profile files and the Git common-directory path, but not every expanded environment value. Changing an SDK path or mount variable can leave an apparently unchanged profile attached to an old container. Recreate explicitly when those inputs change. Linked worktrees also need the shared Git directory mounted at its host path; the launcher supplies that mount.

## Hooks are reviewed snapshots

`dev --install-hooks` copies the reviewed launcher and `tools/repo/git-hook` into this worktree's Git metadata. Switching branches does not update the active hook. Reinstall after reviewing changes to either source.

The hook uses only `--exec-only`: a commit cannot start, rebuild or recreate its environment. Start the worktree container before committing. The fixture-free `check-local` hook and fixture-backed `ci` gate are deliberate separate boundaries, so a local commit does not imply external reader qualification.

## SDK selection is part of the experiment

The sweep wrappers in `tools/repo/` isolate each selected SDK's build output. Their directory keys use list positions, not SDK identities. Keep the HOMES list order stable for saved Criterion baselines, or use fresh output after changing it. A successful build in one position does not qualify a different SDK subsequently assigned to that position.

SHM runtime lookup uses a directory containing only a symlink to the selected reader. Adding the whole vendor library directory can select its older C++ runtime instead of the application's runtime. Loading and downstream dependency propagation need standalone consumers because Cargo supplies a loader environment that can hide deployment failures. See the reader documents for their different SONAME rules.

`cargo doc` still executes build scripts. `--all-features` therefore needs all selected SDKs even when the output is documentation. Do not replace real discovery or linking with a fake reader to make a documentation build succeed.

The fixture gitlink selects metadata and oracles; release tags only locate payload delivery. Installation remains explicit even for cached payloads. Git ignore rules do not protect Docker inputs, so preserve the separate exclusions. `fixture-conformance.md` owns evidence interpretation and corpus changes.

## Release policy

Compatibility includes documented observations, value identity and stop/error semantics, not merely successful consumer compilation. Patch releases preserve those contracts and the Rust floor. A deliberate MSRV increase belongs in package metadata and the changelog and must not be a patch change. Unreleased development checkpoints do not create separate compatibility baselines.

Contract review does not select a version or authorize publication. `release-check` validates the package without publishing; the tag workflow is the publication path. The crates.io and GitHub Release jobs are separate: if only Release creation fails, retry that job rather than repeating crate publication. docs.rs builds asynchronously and is not a release gate.
