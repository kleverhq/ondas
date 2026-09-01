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
- [ ] Add the public devcontainer, pinned Rust toolchains, host launcher, environment examples, and ignore rules.
- [ ] Add the public `just` command surface without test or benchmark recipes.
- [ ] Document every public Rust API item and enable missing-documentation diagnostics.
- [ ] Add local rustdoc instructions and docs.rs metadata.
- [ ] Add GitHub Actions using the public devcontainer and the same `just ci` command.
- [ ] Validate the launcher, recipes, formatting, compilation, Clippy, rustdoc, MSRV, and container recreation behavior.
- [ ] Complete the outcome audit and record exact evidence.

## Surprises & Discoveries

- Observation: The current testing specification is `../ondas_testing_v1.2.md`; the former v1.1 file is no longer present.
  Evidence: `find .. -maxdepth 1 -type f` lists `ondas_testing_v1.2.md`.

- Observation: The crate compiles, but most public operations are deliberately nonfunctional skeletons.
  Evidence: the source contains `unimplemented!`, `panic!`, and the panicking `stub` helper across opening, hierarchy, value, and query methods.

- Observation: The public fixture provider is concrete but not yet consumable by tests because every current oracle is empty and this task explicitly excludes tests.
  Evidence: `../ondas-fixtures-public/catalog.json` identifies `kleverhq.ondas-fixtures-public` version `3.0.0`; 194 fixture sidecars reference about 2.09 GB of artifacts.

- Observation: The official `rust:latest` image currently contains Rust 1.98.0, while `Cargo.toml` declares MSRV 1.85.
  Evidence: `docker run --rm rust:latest rustc --version` printed `rustc 1.98.0 (88d9e12ae 2026-08-18)`.

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

- Decision: Do not create commits while executing this plan.
  Rationale: The working tree already contains the user's uncommitted API-skeleton work. Mixing autonomous commits with that work would obscure ownership and make recovery harder.
  Date/Author: 2026-09-01 / coding agent.

## Outcomes & Retrospective

Implementation is in progress. This section will be replaced with the final observable results, remaining gaps, and validation evidence.

## Context and Orientation

The repository is the Rust library crate at `/home/esynr3z/projects/ondas/ondas-lib`. `Cargo.toml` declares crate version 0.1.0, Edition 2024, `rust-version = "1.85"`, and one dependency, `thiserror`. `src/lib.rs` assembles the public API through explicit re-exports. `src/waveform.rs`, `src/hierarchy.rs`, `src/time.rs`, `src/value.rs`, `src/query.rs`, and `src/error.rs` contain the API skeleton. `README.md` is also the crate-level rustdoc page through `include_str!`.

A devcontainer is a Docker-backed development environment described by `.devcontainer/devcontainer.json`. The Dev Container CLI is the `devcontainer` host command that builds, starts, and executes commands in that environment. `./dev` will be the only supported host wrapper. A worktree is a Git checkout with its own root path; each root must receive a separate labeled container.

`just` is a command runner. The repository's `justfile` will be the stable command surface so contributors and CI do not need to duplicate Cargo flags. MSRV means minimum supported Rust version, currently 1.85 from `Cargo.toml`. Rustdoc is Rust's API documentation generator; docs.rs will generate the published copy after a future crates.io release.

The launcher reads an optional root `.env`. `ONDAS_DEV_CONFIG` selects a devcontainer JSON path relative to the repository root, defaulting to `.devcontainer/devcontainer.json`. `ONDAS_FIXTURES`, when set, points to a host directory containing provider directories such as `kleverhq.ondas-fixtures-public`; the launcher mounts it read-only and exports the corresponding container path.

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

The optional fixture mount is read-only. Changing `.env`, the selected devcontainer profile, public devcontainer files, or the fixture root changes the fingerprint and therefore requires explicit recreation.

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

No expected command includes `cargo test`, `cargo bench`, fixture installation, or publishing.

### Interfaces and Dependencies

The only new project command dependency is `just` version 1.58.0, installed in the container. Docker and Dev Container CLI remain host prerequisites. Rust 1.98.0 is the normal pinned toolchain; Rust 1.85.0 is the MSRV toolchain. Existing crate dependency `thiserror` remains unchanged.

`./dev` accepts an optional leading `--recreate` followed by the command and arguments. It reads only `ONDAS_DEV_CONFIG` and `ONDAS_FIXTURES` from its supported host configuration. It invokes the selected command directly through `devcontainer exec` and returns that command's exit status.

Revision note (2026-09-01): Initial self-contained plan created after repository, fixture corpus, and project specification review. The scope deliberately excludes tests and runtime implementation per the user's request.
