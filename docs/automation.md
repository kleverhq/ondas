# Development and automation

## Environment

Linux `x86_64-unknown-linux-gnu` is the supported target for the public container,
CI, docs.rs and release checks. Other platforms are best effort.

Keep Git, editors, agents, signing and credentials on the host. Run builds, tests,
formatting, benchmarks and project tools in the devcontainer. The host needs Git,
Docker with a working daemon, the Dev Container CLI, Bash and standard Linux
utilities.

From the repository root, after initializing the pinned fixture submodule on the host and installing its payloads for full CI:

```sh
./dev --install-hooks
./dev just --list
./dev just ci
./dev just msrv
./dev just docs
```

`justfile` defines the recipes and their checks. Prefer recipes; use
`./dev <command> ...` when none exists, for example
`./dev cargo test --doc --locked`. Local API docs are in
`target/doc/ondas/index.html`.

The public configuration is `.devcontainer/devcontainer.json` and its Dockerfile.
Install only necessary, freely redistributable tools. Vendor SDKs, licenses,
private networks and host-dependent reader discovery belong outside this image.
Recipes use `ONDAS_IN_CONTAINER` to reject host execution.

## Launcher and local configuration

`dev` resolves the Git worktree, starts its container if needed and runs the
command at the matching relative working directory. Each worktree has its own
container identity. Linked worktrees mount their shared Git directory at its
absolute host path. The launcher preserves arguments, streams, exit status and
cancellation.

An optional root `.env` supplies local settings:

```dotenv
ONDAS_DEV_CONFIG=.devcontainer.local/devcontainer.json
```

`ONDAS_DEV_CONFIG` must be relative to, and inside, the worktree root.
The fixture submodule is available through the ordinary workspace mount.

The host sources `.env` as Bash. Treat it as trusted executable configuration,
not data, and do not load it again through `just`. Profiles forward selected
values through mounts, `containerEnv` or `remoteEnv`, not wholesale secret exports.

Recreate explicitly after changing environment, mounts or image inputs:

```sh
./dev --recreate just ci
```

The launcher fingerprints profile files and the Git common-directory path, not every
expanded environment value. Recreation removes the container; keep important data
in the checkout or persistent mounts rather than its writable layer.

## Pre-commit

Run `./dev --install-hooks` on the host in each checkout or linked worktree. It
needs no container, host Python or host Pre-commit. Installation copies the
reviewed `dev` and `tools/repo/git-hook` into the worktree's Git `ondas-hooks/`
directory, enables worktree configuration and sets `core.hooksPath`.

Installation is idempotent and refuses unrelated hooks paths. Worktrees have
separate copies; switching branches does not replace active hook code. Reinstall
after reviewing launcher or dispatcher changes. Do not install hooks at container
startup or run `pre-commit install` inside the container.

The host dispatcher maps the worktree, Git/common directories and exact
`GIT_INDEX_FILE`, clears inherited host Git environment and invokes the installed
launcher's `--exec-only` mode. Container Pre-commit isolates staged changes and
runs `.pre-commit-config.yaml`. Its `just check-local` gate needs no fixtures and
never installs them or runs conformance.

Start the container with `./dev true` or another `./dev` command before committing.
Hooks never start, rebuild or recreate it. A missing, stopped or stale container
fails with manual recovery instructions. Do not bypass hooks without the user's
explicit request.

`./dev just pre-commit` checks all tracked files. `./dev just tools-test` uses
temporary repositories and fake container commands to test hook installation,
index/worktree mapping and dispatch without Docker. See `tools/repo/README.md`.

## Proprietary environments

Select an ignored `.devcontainer.local/` profile through `ONDAS_DEV_CONFIG`.
Prefer the public Dockerfile with explicit runtime mounts, network settings and
environment. Mount SDKs and credentials read-only unless an operation needs writes.
Use a local Dockerfile only for requirements such as packages or linker setup
that runtime configuration cannot supply. Keep the public workspace layout, user
and command-entry model; a little profile JSON duplication is simpler than a
configuration generator.

`just.local` is reserved for private recipes when the public `justfile` enables
its optional import. It adds commands, not alternate meanings: `just ci` must not
change with installed vendor tools. Select SDK versions and environments explicitly.
Use separate `CARGO_TARGET_DIR` values when build outputs depend on SDK headers
or ABI.

The launcher does not discover EDA installations, select versions, configure
licenses or branch on backend names. Profiles and recipes own those choices.
Private CI topology is outside the public contract.

## Fixtures, tools and privacy

Initialize the `fixtures/` Git submodule on the host and install payloads explicitly
with `./dev just fixtures-install`. The workspace mount supplies fixtures to the
container. The launcher does not select corpus revisions or install data;
[fixture integration](fixtures.md) defines setup and consumer checks.

Exclude `.env`, `just.local`, `.devcontainer.local/`, `tools.local/`, downloaded
waveform payloads, SDKs, credentials and infrastructure details from
Git and Docker contexts. The submodule tracks fixture metadata, ignores downloaded
waveforms, and is excluded in full from Docker contexts and the crate package.
Git ignore rules alone do not exclude Docker inputs.
Default checks need neither credentials nor a license network.

Use ignored `tmp/` for scratch and logs without deleting others' work. Tracked
temporary work belongs in `docs/wip/yymmdd-slug/`. Move durable conclusions into
their owning docs and remove task directories before merging into `main`; retain
`docs/wip/AGENTS.md`. Do not automate this cleanup rule in hooks or CI.

Keep public recipes stable and reproducible, and request private suites separately.
Do not add empty recipes that imply coverage. Benchmark compilation and smoke runs
can be checks, but wall-clock timings cannot be gates.

Short wrappers belong in `justfile`. Put nontrivial public helpers in
`tools/<name>/` with an entry point, tests, a recipe and a README covering purpose,
inputs/outputs, modified files, dependencies, usage and limits. Use shell for thin
wrappers; use a testable language for parsing, checksums, filesystem validation and
subprocess handling. Keep private helpers in `tools.local/` and avoid a shared
framework until real common logic exists.

Validation, binding generation, environment checks and release helpers are tools.
Decoding, hierarchy, values, projections and queries belong in the library.
Materialization must never happen implicitly during a query or fixture test.

## CI and Rust versions

GitHub Actions uses the public devcontainer and contributor `just ci`, without
local `.env` or private profiles. It runs on pushes to `main`, pull requests and
`v*` tags. Tag builds require a stable `vX.Y.Z` matching the package version and a
commit already merged into `main`.
BuildKit's GitHub Actions cache reuses Dockerfile layers. The runtime config uses
the loaded image with the public settings; no registry publication credentials
are needed.

CI checks out the pinned fixture submodule and keys its payload cache by OS and
that Git entry's commit SHA. Only waveform files and directories are cached;
sidecars, scripts, schemas and Git state always come from the selected checkout.
CI runs `just fixtures-install` even on cache hits, then `just ci` and `just msrv`.
The fixture installer verifies payload sizes and hashes. Missing assets and
corrupt downloads fail. GitHub's read-only token supplies API access; fork PRs
need no additional secrets.

Contributors use the same explicit installation command. Neither `check-local`
nor conformance downloads implicitly. After a parent branch change, run
`git submodule update --init fixtures` on the host to select its pinned corpus.
Missing required inputs fail. Release metadata and the crates.io package are
verified without publishing during `just ci`.

`rust-toolchain.toml` pins development Rust; `Cargo.toml`'s `rust-version` sets the
MSRV used by `just msrv`. Keep container installation consistent with both. Choose
the MSRV from supported dependencies, language/library requirements and any
justified project floor, not just current stable or an old dependency snapshot.
Check proprietary configurations at that same MSRV in their explicit environment.
Record intentional increases in package metadata and the changelog, never in a
patch release.

## Documentation and releases

Rustdoc owns API contracts and examples; `docs/` owns concepts and policies.
Build docs with warnings denied. `package.metadata.docs.rs` selects features and
targets; `just docs` reproduces that build. docs.rs builds after crates.io
publication, so no separate site or publishing job is needed.

Public checks and docs.rs use the default nonvendor feature set. Optional backend
features are additive, but `--all-features` includes SDK-dependent builds and is
not a public CI requirement. `cargo doc` still executes build scripts; do not
substitute a fake reader or silently bypass SDK discovery for documentation.
Successful docs do not establish native linking or runtime compatibility.

`just ci-fsdb` explicitly enables `fsdb-lib`, checks development Rust and MSRV,
and runs real FSDB conformance. Supply a complete read-only SDK mount and
`VERDI_HOME` through the ignored local profile. Install the pinned fixtures
explicitly before running conformance; tests do not download data. See
[fsdb-lib](fsdb-lib.md) for the source-build deployment contract and
[fixtures](fixtures.md) for installation and validation.

Compatibility covers released public signatures and documented observations,
including value identity and query semantics, not just whether consumers compile.
Breaking changes require a minor version increase before 1.0, or a major increase
afterward, with migration notes and contract tests. Patch releases preserve those
contracts and the supported Rust floor. Unreleased development checkpoints are
not separate compatibility baselines. Contract review does not itself bump a
version, tag, publish or authorize a release.

A release consists of a crates.io package, `vX.Y.Z` tag and GitHub Release, without
a binary matrix, release assets or Pages site. Preparation updates the package
version, affected lockfile entries and dated changelog section. `just ci` includes
`just release-check`: version/changelog validation, `cargo package --list` and
`cargo publish --dry-run`. Run MSRV and SDK-backed checks separately before
release. These local commands never publish.

After the release PR is merged and verified, push its `vX.Y.Z` tag. The CI
workflow repeats the public checks, then publishes the crate using the repository
secret `CRATES_IO_TOKEN`. Only tag builds receive this token. A dependent job
creates the GitHub Release from that version's changelog notes, without binary
assets. If only GitHub Release creation fails, rerun the failed job rather than
the successful crate publication. docs.rs builds asynchronously; the release
does not wait for it.
