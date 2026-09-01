# Establish the documented Ondas development and CI path

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with the repository agent's `exec-plan` skill.

## Purpose / Big Picture

After this change, a contributor with Linux, Docker, and the Dev Container CLI can enter one reproducible public development environment with `./dev`, run the supported quality gate with `./dev just ci`, and generate useful API documentation with `./dev just docs`. GitHub Actions will run the same public gate. The gate will honestly check formatting, compilation, Clippy, and rustdoc only; it will not imply that the API skeleton works at runtime.

The public Rust API will have documentation for every exported item. The generated entry page will remain `README.md`, while module and item comments explain the intended format-independent waveform model from the project specification.

## Non-Goals

This work does not add or run tests. It does not add `test`, `conformance`, or `tools-test` recipes that could misleadingly succeed with no coverage. It does not implement waveform backends, paths, hierarchy, values, or queries. It does not add Criterion, benchmark targets, fixture validation or download logic, `fixtures.lock.toml`, proprietary development profiles, vendor discovery, release publishing, changelog tooling, or GitHub Release automation.

The sibling fixture provider is not copied into the image. Its 194 waveform artifacts total roughly 2.09 GB and remain an optional host mount selected by `ONDAS_FIXTURES`. The source-catalog ZIP in the parent directory is research input and is not part of this crate's automation.

## Progress

- [x] (2026-09-01 14:18Z) Read the four current project specifications, all current crate files, all sibling fixture-provider metadata and scripts, and inventory the source-catalog ZIP.
- [x] (2026-09-01 14:18Z) Define the minimal honest automation and documentation scope in this ExecPlan.
- [x] (2026-09-01 14:54Z) Add the public devcontainer, pinned Rust toolchains, host launcher, environment examples, and ignore rules.
- [x] (2026-09-01 14:54Z) Add the public `just` command surface without test or benchmark recipes.
- [x] (2026-09-01 14:54Z) Document every public Rust API item and enable missing-documentation diagnostics.
- [x] (2026-09-01 14:54Z) Add local rustdoc instructions and docs.rs metadata.
- [x] (2026-09-01 14:54Z) Add GitHub Actions using the public devcontainer and the same `just ci` command.
- [x] (2026-09-01 14:56Z) Validate the launcher, recipes, formatting, compilation, Clippy, rustdoc, MSRV, nested cwd, exit status, marker, invalid config, fingerprint refusal, recreation, fixture mount, and separate temporary Git worktree container.
- [x] (2026-09-01 14:56Z) Complete the outcome audit and record exact evidence.

## Surprises & Discoveries

- Observation: The current testing specification is `../ondas_testing_v1.2.md`; the former v1.1 file is no longer present.
  Evidence: `find .. -maxdepth 1 -type f` lists `ondas_testing_v1.2.md`.

- Observation: The crate compiles, but most public operations are deliberately nonfunctional skeletons.
  Evidence: the source contains `unimplemented!`, `panic!`, and the panicking `stub` helper across opening, hierarchy, value, and query methods.

- Observation: The public fixture provider is concrete but not yet consumable by tests because every current oracle is empty and this task explicitly excludes tests.
  Evidence: `../ondas-fixtures-public/catalog.json` identifies `kleverhq.ondas-fixtures-public` version `3.0.0`; 194 fixture sidecars reference about 2.09 GB of artifacts.

- Observation: The official `rust:latest` image currently contains Rust 1.98.0, while `Cargo.toml` declares MSRV 1.85.
  Evidence: `docker run --rm rust:latest rustc --version` printed `rustc 1.98.0 (88d9e12ae 2026-08-18)`.

- Observation: Dev Container CLI ignored a fixed `workspaceFolder` that did not match its inferred bind target, so container startup failed before command execution.
  Evidence: the first `./dev just check` mounted the checkout at `/workspaces/ondas-lib` but attempted to start in nonexistent `/workspaces/ondas`. Using `${localWorkspaceFolderBasename}` aligns both paths.

- Observation: Dev Container CLI 0.86.0 does not accept a dynamic `readonly` field in its `--mount` grammar.
  Evidence: `devcontainer up` rejected `type=bind,...,readonly`; the project specification requires the fixture root to be mounted but does not require that mount to be read-only.

## Decision Log

- Decision: Keep modules private and retain explicit root re-exports as the public API.
  Rationale: Documentation work must not change downstream paths such as `ondas::Waveform`.
  Date/Author: 2026-09-01 / coding agent.

- Decision: Pin the normal development toolchain to Rust 1.98.0 and install Rust 1.85.0 for the explicit MSRV recipe.
  Rationale: The automation specification asks for a pinned current stable toolchain and a separate oldest-supported check. Edition 2024 requires at least Rust 1.85.
  Date/Author: 2026-09-01 / coding agent.

- Decision: Use a small `justfile` with `fmt`, `fmt-check`, `lint`, `check`, `docs`, `msrv`, and `ci` only.
  Rationale: Empty test, conformance, tool, or benchmark recipes would report success without exercising anything and conflict with the user's explicit no-tests scope.
  Date/Author: 2026-09-01 / coding agent.

- Decision: Mount `ONDAS_FIXTURES` only when explicitly configured and include the selected mount in the container fingerprint.
  Rationale: Fixtures are external and large. A changed mount must not silently reuse a container created with a different filesystem view.
  Date/Author: 2026-09-01 / coding agent.

- Decision: Create conventional commits at coherent milestones.
  Rationale: The user explicitly authorized as many commits as needed after the initial API skeleton and documentation were reviewed.
  Date/Author: 2026-09-01 / coding agent.

- Decision: Use the Dev Container CLI's supported writable bind mount for optional fixtures.
  Rationale: CLI 0.86.0 rejects a `readonly` option on dynamic `--mount`, and Automation v1.1 requires mounting the root but does not require read-only fixture access. The launcher itself never writes fixture files.
  Date/Author: 2026-09-01 / coding agent.

## Outcomes & Retrospective

The plan is complete. A fresh checkout now has one public host entry point, `./dev`, backed by the pinned Rust 1.98.0 devcontainer. The container also carries Rust 1.85.0, and `just msrv` derives that promised version from `Cargo.toml`. The public recipes cover only formatting, Clippy, library compilation, and rustdoc, as intended.

Every public API item has rustdoc and `warn(missing_docs)` is enabled. `./dev just docs` generated `target/doc/ondas/index.html` with warnings denied. GitHub Actions uses `.devcontainer/devcontainer.json` and runs `just ci && just msrv`; actionlint accepted the workflow.

The launcher was exercised from the repository root and `src/`, propagated exit status 23 unchanged, rejected an invalid profile, rejected a changed configuration until `--recreate`, mounted an explicitly configured fixture root, and assigned a separate labeled container to a temporary detached Git worktree. ShellCheck accepted the final script. No test file, benchmark target, fixture tool, proprietary profile, release workflow, or runtime implementation was added.

The main implementation lesson was to follow the Dev Container CLI's inferred workspace mount path with `${localWorkspaceFolderBasename}` rather than imposing a fixed path. Dynamic fixture mounts are writable because CLI 0.86.0 does not expose a read-only bind option through `devcontainer up --mount`; the launcher does not itself write to the mount.

## Context and Orientation

The repository is the Rust library crate at `/home/esynr3z/projects/ondas/ondas-lib`. `Cargo.toml` declares crate version 0.1.0, Edition 2024, `rust-version = "1.85"`, and one dependency, `thiserror`. `src/lib.rs` assembles the public API through explicit re-exports. `src/waveform.rs`, `src/hierarchy.rs`, `src/time.rs`, `src/value.rs`, `src/query.rs`, and `src/error.rs` contain the API skeleton. `README.md` is also the crate-level rustdoc page through `include_str!`.

A devcontainer is a Docker-backed development environment described by `.devcontainer/devcontainer.json`. The Dev Container CLI is the `devcontainer` host command that builds, starts, and executes commands in that environment. `./dev` will be the only supported host wrapper. A worktree is a Git checkout with its own root path; each root must receive a separate labeled container.

`just` is a command runner. The repository's `justfile` will be the stable command surface so contributors and CI do not need to duplicate Cargo flags. MSRV means minimum supported Rust version, currently 1.85 from `Cargo.toml`. Rustdoc is Rust's API documentation generator; docs.rs will generate the published copy after a future crates.io release.

The launcher reads an optional root `.env`. `ONDAS_DEV_CONFIG` selects a devcontainer JSON path relative to the repository root, defaulting to `.devcontainer/devcontainer.json`. `ONDAS_FIXTURES`, when set, points to a host directory containing provider directories such as `kleverhq.ondas-fixtures-public`; the launcher mounts it and exports the corresponding container path.

## Open Questions

There are no blocking product questions. Release automation, fixture locking, conformance testing, and proprietary profiles remain intentionally deferred until their implementations exist.

## Plan of Work

First, create `rust-toolchain.toml`, `.devcontainer/Dockerfile`, and `.devcontainer/devcontainer.json`. The image will pin Rust 1.98.0, install rustfmt, Clippy, Rust 1.85.0, and pinned `just`, and define a non-root development user. No simulator, Python tool, vendor library, or fixture payload belongs in the image.

Next, create executable `dev`. It will locate the current Git worktree root, load `.env` if present, validate Docker and the Dev Container CLI, resolve the selected configuration below the worktree root, and preserve the caller's relative directory. It will label containers with a worktree hash and a configuration fingerprint. A changed configuration or fixture mount will produce an error instructing the caller to pass `--recreate`; that flag will remove only containers carrying the same Ondas worktree label. It will pass the command and exit status through `devcontainer exec` without evaluating command text.

Then create `justfile`, `.env.example`, and `.dockerignore`, and expand `.gitignore`. The command recipes will require `ONDAS_IN_CONTAINER=1`. `ci` will depend on `fmt-check`, `lint`, `check`, and `docs`. `msrv` will be separate. There will be no call to `cargo test`.

Then add concise `//!` and `///` comments to all public API items and exported fields. The comments will describe the intended contract from `../ondas_model_v2.1.md` without claiming the backends are implemented. `src/lib.rs` will enable `warn(missing_docs)`. `README.md` will explain the current skeleton status and the `./dev just ...` workflow. `Cargo.toml` will ask docs.rs to build all currently available features for `x86_64-unknown-linux-gnu`.

Finally add `.github/workflows/ci.yml`. On pushes and pull requests it will check out the repository and use the public devcontainer to run `just ci` followed by `just msrv`. It will not read `.env`, mount private data, detect vendor libraries, run tests, or publish anything.

### Concrete Steps

All commands run from `/home/esynr3z/projects/ondas/ondas-lib`.

Create the files described above, then make the launcher executable:

    chmod +x dev

Build and exercise the public environment:

    ./dev just fmt-check
    ./dev just lint
    ./dev just check
    ./dev just docs
    ./dev just msrv
    ./dev just ci

A successful command exits with status 0. The documentation entry page must exist at `target/doc/ondas/index.html` on the mounted workspace.

Verify that commands outside the container fail clearly:

    just check

The expected error tells the user to run `./dev just check`.

Verify configuration-change protection by starting the container, changing a file under `.devcontainer`, and running another command. The launcher must refuse reuse and mention `./dev --recreate`. Restore the file and use `./dev --recreate just check` to prove recovery. Do not leave the temporary modification in the final tree.

Inspect the GitHub Actions workflow syntactically and ensure its only project gate commands are `just ci` and `just msrv` inside the public devcontainer.

### Validation and Acceptance

The work is accepted when a host with only Git, Docker, and the Dev Container CLI can run `./dev just ci` successfully; `./dev just msrv` succeeds using Rust 1.85.0; `./dev just docs` generates `target/doc/ondas/index.html`; Clippy and rustdoc treat warnings as failures; every public API item has a rustdoc comment; and no test, benchmark, fixture downloader, vendor integration, or release publisher has been added.

The launcher must preserve nested working directories, command exit codes, and separate worktree identity. Missing Docker, missing Dev Container CLI, invalid configuration, invalid fixture root, and stale container configuration must each produce a concise actionable error rather than a stack trace or silent fallback.

The CI workflow must use `.devcontainer/devcontainer.json` and run the same `just ci` semantics as local development. A green result only claims static quality and documentation for the API skeleton.

### Idempotence and Recovery

Docker builds, `devcontainer up`, Cargo checks, and rustdoc generation are idempotent. `./dev --recreate` removes only the container selected by the current worktree label and then rebuilds it; it never removes source files, fixtures, images, or unrelated containers. If image construction fails, rerun the same command after correcting the reported Docker or network problem. Generated `target/` content remains ignored.

The optional fixture mount is never copied into the image or modified by the launcher. Changing the selected devcontainer profile, public devcontainer files, or the fixture root changes the fingerprint and therefore requires explicit recreation. Unrelated `.env` values do not.

### Artifacts and Notes

Expected final command surface:

    ./dev just fmt
    ./dev just fmt-check
    ./dev just lint
    ./dev just check
    ./dev just docs
    ./dev just msrv
    ./dev just ci

Expected public CI core:

    just ci
    just msrv

Final local evidence:

    ./dev just ci
    Finished `dev` profile ...
    Generated /workspaces/ondas-lib/target/doc/ondas/index.html

    ./dev just msrv
    cargo +1.85.0 check --locked --lib --all-features
    Finished `dev` profile ...

    shellcheck dev
    actionlint .github/workflows/ci.yml
    # both exited 0

No expected command includes `cargo test`, `cargo bench`, fixture installation, or publishing.

### Interfaces and Dependencies

The only new project command dependency is `just` version 1.58.0, installed in the container. Docker and Dev Container CLI remain host prerequisites. Rust 1.98.0 is the normal pinned toolchain; Rust 1.85.0 is the MSRV toolchain. Existing crate dependency `thiserror` remains unchanged.

`./dev` accepts an optional leading `--recreate` followed by the command and arguments. It reads only `ONDAS_DEV_CONFIG` and `ONDAS_FIXTURES` from its supported host configuration. It invokes the selected command directly through `devcontainer exec` and returns that command's exit status.

Revision note (2026-09-01): Initial self-contained plan created after repository, fixture corpus, and project specification review. The scope deliberately excludes tests and runtime implementation per the user's request.

Revision note (2026-09-01 14:54Z): Recorded completed implementation milestones, launcher discoveries, the fixture-mount decision, and remaining validation work.

Revision note (2026-09-01 14:56Z): Marked validation complete and recorded final commands, worktree behavior, generated documentation, exclusions, and implementation lessons.
