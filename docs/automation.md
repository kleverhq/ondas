# Development and automation

## Environment

Linux `x86_64-unknown-linux-gnu` is the supported target for the public container,
CI, docs.rs and release checks. Other platforms are best effort.

Keep Git, editors, agents, signing and credentials on the host. Run builds, tests,
formatting, benchmarks and project tools in the devcontainer. The host needs Git,
Docker with a working daemon, the Dev Container CLI, Bash and standard Linux
utilities.

From the repository root, after installing the locked fixtures for full CI:

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
ONDAS_FIXTURES=/absolute/host/path/to/fixtures
```

`ONDAS_DEV_CONFIG` must be relative to, and inside, the worktree root.
`ONDAS_FIXTURES` is an absolute host directory. The launcher mounts it and passes
its container path under the same variable name; tests must not reuse the host
path.

The host sources `.env` as Bash. Treat it as trusted executable configuration,
not data, and do not load it again through `just`. Profiles forward selected
values through mounts, `containerEnv` or `remoteEnv`, not wholesale secret exports.

Recreate explicitly after changing environment, mounts or image inputs:

```sh
./dev --recreate just ci
```

The launcher fingerprints profile files and the fixture-root path, not every
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

The launcher mounts the fixture root; it does not download or generate data,
validate catalogs, update hashes or change locks. Fixture tools run inside the
container under the [fixture contract](fixtures.md).

Exclude `.env`, `just.local`, `.devcontainer.local/`, `tools.local/`, materialized
fixtures, SDKs, credentials, private sidecars and infrastructure details from both
Git and Docker contexts. Git ignore rules alone do not exclude Docker inputs.
Public runs must need neither private credentials nor a license network.

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
local `.env` or private profiles. It runs on pushes to `main` and pull requests.
BuildKit's GitHub Actions cache reuses Dockerfile layers. The runtime config uses
the loaded image with the public settings; no registry publication credentials
are needed.

The fixture cache key contains the OS and lock-file hash. CI sets
`ONDAS_FIXTURES`, runs `just fixtures-install` even on cache hits, then
`just ci && just msrv`. Installation clones the exact `v<version>` tag from
`kleverhq/ondas-fixtures`, checks checkout/catalog identity and invokes its
installer without `--ignore-missing`. It verifies cached payload sizes and hashes.
Missing assets and corrupt downloads fail; there is no latest-tag fallback or
cross-version cache restore. GitHub's read-only token supplies API access, and
fork PRs need no private secrets.

Local installation uses the same explicit target. Neither CI nor `check-local`
downloads implicitly. Installation rejects existing checkouts at another revision
or with tracked edits rather than resetting them. Use another fixture root or
manage that checkout deliberately. Missing required inputs fail.

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

Prefer public all-features builds without SDK installations and report absent
runtimes when opening a source. `cargo doc` still builds dependencies and executes
build scripts. If needed, a small `DOCS_RS`/`cfg(docsrs)` path can bypass discovery
while preserving the API; do not create a fake reader. If a dependency cannot
support this, document an explicit open-source feature set. Successful docs do
not establish private linking, licensing or runtime compatibility.

A release consists of a crates.io package, `vX.Y.Z` tag and GitHub Release, without
a binary matrix, release assets or Pages site. Preparation updates the package
version, affected lockfile entries and changelog. Checks must not publish: run
public CI, MSRV, docs, version/changelog checks, `cargo package --list` and
`cargo publish --dry-run`. Available commands remain defined by `justfile` and
`.github/workflows/`.

Publish from a version tag after merge and verification. Publish the crate before
creating the GitHub Release from its changelog. Prefer Actions OIDC Trusted
Publishing, once configured, over a long-lived registry token. docs.rs builds
asynchronously; the release does not wait for it.
