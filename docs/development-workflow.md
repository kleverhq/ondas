# Worktrees, native environments and release boundaries

`dev`, `justfile`, `.devcontainer/` and `.github/workflows/ci.yml` define setup and checks. Public checks use the container without vendor SDKs. Optional SDK features must not make this gate depend on local installations.

Linux `x86_64-unknown-linux-gnu` is the supported public target. Other platforms have no guaranteed support.

After public documentation changes, run `just ci`, `just msrv` and `cargo test --doc --locked` through `./dev`. Before `ci`, initialize the pinned `fixtures` submodule on the host and run `fixtures-install` explicitly. Missing inputs must fail. The package dry-run requires committed changes.

## Configuration is executable and worktree-local

The host sources `.env` as Bash, so this file can execute commands. `ONDAS_DEV_CONFIG` selects a profile inside the worktree. Forward only the environment and mounts that profile needs. Keep private infrastructure outside the public image and Docker context.

The launcher fingerprints profile files and the Git common-directory path. The fingerprint does not cover all expanded environment values. A changed SDK path or mount variable can therefore leave the old container in use. Recreate the container when these inputs change. Linked worktrees also need the shared Git directory mounted at its host path. The launcher supplies this mount.

## Remove an installed Ondas hook

An earlier Ondas setup installed a copy of the hook in an `ondas-hooks` directory inside the Git metadata. Git keeps using that copy after you update the checkout. Run this command from each checkout and linked worktree to see the configured hook paths and their scopes.

```sh
git config --show-scope --get-all core.hooksPath
```

If a `worktree` entry points to the directory installed by Ondas, remove that exact value with the command below. A linked worktree can show a copy of another checkout's hook path. Replace the example path with the path from the entry.

```sh
git config --worktree --fixed-value --unset-all core.hooksPath '/path/to/ondas-hooks'
```

Run `./dev just check-local` for an offline check. The pull request workflow runs this recipe as part of `just ci`.

## SDK selection is part of the experiment

When comparing baseline and candidate builds from different source trees, give each tree and SDK/feature configuration a separate `CARGO_TARGET_DIR`. Use only the executable reported by a successful Cargo build. Record its path and content hash with the source revision and source content hashes. After a failed build, do not run an executable left by an earlier build. Reuse each configuration's build cache; do not clear all caches by default.

The sweep wrappers in `tools/repo/` keep each selected SDK's build output separate. Output directories use list positions, not SDK identities. Keep the HOMES list order stable for saved Criterion baselines. Use fresh output if that order changes. A successful build at one position does not qualify another SDK later assigned to that position.

SHM runtime lookup uses a directory with only a symlink to the selected reader. The full vendor library directory can select its older C++ runtime instead of the application's runtime. Use standalone consumers to check loading and downstream dependencies. Cargo supplies a loader environment that can hide deployment failures. The reader documents explain their different SONAME rules.

`cargo doc` executes build scripts. With `--all-features`, it needs all selected SDKs. Do not use a fake reader in place of real discovery or linking to build documentation.

The fixture gitlink selects metadata and oracles. Release tags only locate payload delivery. Installation remains explicit even for cached payloads. Git ignore rules do not exclude Docker inputs. Keep the Docker exclusions as well. `fixture-conformance.md` explains evidence and corpus changes.

## Release policy

Compatibility covers documented observations, value identity and stop/error semantics. Successful consumer compilation alone does not prove compatibility. Patch releases preserve these contracts and the Rust floor. Record an MSRV increase in package metadata and the changelog. Do not increase it in a patch release. Unreleased development checkpoints do not create separate compatibility baselines.

Run MSRV and SDK-backed checks separately before release. The public and tag gates use default features. They do not qualify native readers.

Contract review does not select a version or authorize publication. `release-check` validates the package without publishing. The tag workflow publishes it. The crates.io and GitHub Release jobs are separate. If only Release creation fails, retry that job without repeating crate publication. docs.rs builds asynchronously and is not a release gate.
