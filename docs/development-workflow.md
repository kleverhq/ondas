# Worktrees, native environments and release boundaries

`dev`, `justfile`, `.devcontainer/` and `.github/workflows/ci.yml` define setup and checks. Public checks use the container without vendor SDKs. Optional SDK features must not make this gate depend on local installations.

Linux `x86_64-unknown-linux-gnu` is the supported public target. Other platforms have no guaranteed support.

After public documentation changes, run `just ci`, `just msrv` and `cargo test --doc --locked` through `./dev`. Before `ci`, initialize the pinned `fixtures` submodule on the host and run `fixtures-install` explicitly. Missing inputs must fail. The package dry-run requires committed changes.

## Configuration is executable and worktree-local

The host sources `.env` as Bash, so this file can execute commands. `ONDAS_DEV_CONFIG` selects a profile inside the worktree. Forward only the environment and mounts that profile needs. Keep private infrastructure outside the public image and Docker context.

The launcher fingerprints profile files and the Git common-directory path. The fingerprint does not cover all expanded environment values. A changed SDK path or mount variable can therefore leave the old container in use. Recreate the container when these inputs change. Linked worktrees also need the shared Git directory mounted at its host path. The launcher supplies this mount.

## Hooks are reviewed snapshots

Install reviewed hooks in each new checkout or worktree with `./dev --install-hooks`. This copies the launcher and `tools/repo/git-hook` into the worktree's Git metadata. Switching branches does not update the active hook. Reinstall after reviewing changes to either source.

The hook uses only `--exec-only`. A commit cannot start, rebuild or recreate its container. Start the worktree container before committing. Do not bypass hooks unless the user explicitly requests it. The hook runs `check-local` without fixtures. The `ci` gate also runs external conformance. A local commit therefore does not prove reader conformance.

## SDK selection is part of the experiment

The sweep wrappers in `tools/repo/` keep each selected SDK's build output separate. Output directories use list positions, not SDK identities. Keep the HOMES list order stable for saved Criterion baselines. Use fresh output if that order changes. A successful build at one position does not qualify another SDK later assigned to that position.

SHM runtime lookup uses a directory with only a symlink to the selected reader. The full vendor library directory can select its older C++ runtime instead of the application's runtime. Use standalone consumers to check loading and downstream dependencies. Cargo supplies a loader environment that can hide deployment failures. The reader documents explain their different SONAME rules.

`cargo doc` executes build scripts. With `--all-features`, it needs all selected SDKs. Do not use a fake reader in place of real discovery or linking to build documentation.

The fixture gitlink selects metadata and oracles. Release tags only locate payload delivery. Installation remains explicit even for cached payloads. Git ignore rules do not exclude Docker inputs. Keep the Docker exclusions as well. `fixture-conformance.md` explains evidence and corpus changes.

## Release policy

Compatibility covers documented observations, value identity and stop/error semantics. Successful consumer compilation alone does not prove compatibility. Patch releases preserve these contracts and the Rust floor. Record an MSRV increase in package metadata and the changelog. Do not increase it in a patch release. Unreleased development checkpoints do not create separate compatibility baselines.

Run MSRV and SDK-backed checks separately before release. The public and tag gates use default features. They do not qualify native readers.

Contract review does not select a version or authorize publication. `release-check` validates the package without publishing. The tag workflow publishes it. The crates.io and GitHub Release jobs are separate. If only Release creation fails, retry that job without repeating crate publication. docs.rs builds asynchronously and is not a release gate.
