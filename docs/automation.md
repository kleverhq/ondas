# Development and Automation

## Environment and Commands

The supported platform is Linux, with `x86_64-unknown-linux-gnu` as the public
container, CI, docs.rs, and release-check target. Other platforms are best effort,
not an implied support promise.

Git, editors, agents, signing, and credentials stay on the host. Builds, tests,
formatting, benchmarks, and project tools run in the devcontainer. The host needs
Git, Docker with a working daemon, the Dev Container CLI, Bash, and the ordinary
Linux utilities used by `dev`.

From the repository root:

```sh
./dev --install-hooks
./dev just --list
./dev just ci
./dev just msrv
./dev just docs
```

`justfile` is the executable source of truth for available recipes and their gate
coverage. Prefer a recipe over its underlying command when one exists. Use
`./dev <command> ...` for direct commands without a recipe, such as
`./dev cargo test --doc --locked`. Open `target/doc/ondas/index.html` after the
local documentation build.

The public environment is defined by `.devcontainer/devcontainer.json` and its
Dockerfile. Keep installed tools minimal and freely redistributable. Do not add
vendor SDKs, license settings, private network requirements, or host-dependent
reader discovery to the public image. The `ONDAS_IN_CONTAINER` marker lets
recipes reject unsupported host execution with an actionable message.

## Launcher and Local Configuration

`dev` resolves the current Git worktree, starts its container as needed, and runs
the requested command at the corresponding relative working directory. Each
worktree has its own container identity. A linked worktree's shared Git directory
is mounted at its absolute host path so Git metadata and indexes remain accessible
inside the container. Command arguments, standard streams,
exit status, and cancellation belong to the launcher's process boundary.

The default configuration is `.devcontainer/devcontainer.json`. An optional root
`.env` supplies host-local settings, for example:

```dotenv
ONDAS_DEV_CONFIG=.devcontainer.local/devcontainer.json
ONDAS_FIXTURES=/absolute/host/path/to/fixtures
```

`ONDAS_DEV_CONFIG` is relative to the worktree root and must stay inside it.
`ONDAS_FIXTURES` is an absolute host directory. The launcher mounts the fixture
root and passes its container-side location to the command under the same
variable name. Test and benchmark code must not reuse the host path.

The launcher sources `.env` as Bash on the host; it is trusted local executable
configuration, not an untrusted data file. Do not load it again through `just` in
the container. Profiles explicitly choose forwarded values with mounts,
`containerEnv`, or `remoteEnv` rather than exporting secrets wholesale.

Configuration changes require explicit recreation rather than silently deleting
a working container:

```sh
./dev --recreate just ci
```

Use recreation after changing profile-dependent environment, mount settings, or
image inputs. Do not rely on fingerprinting to detect every external input: the
launcher fingerprints profile files and the fixture-root path, not all expanded
environment variables. Recreation removes the worktree's container, so retain
important data in the checkout or other persistent mounts rather than its
writable layer.

## Host Pre-commit Hook

Install reviewed hook copies explicitly from the host with `./dev --install-hooks`
in each checkout or linked worktree. Installation needs no container, host Python,
or host Pre-commit. It copies `dev` and `tools/repo/git-hook` into `ondas-hooks/`
under that worktree's Git directory, enables Git's worktree configuration, and
sets worktree-local `core.hooksPath`.

Installation is idempotent and refuses to replace an unrelated configured hooks
path. Main and linked worktrees have separate copies; branch switches do not
replace active hook code. Reinstall deliberately after reviewing launcher or
dispatcher changes. Do not install hooks implicitly during container startup or
through `pre-commit install` inside the container.

Git invokes the installed hook on the host. The dispatcher maps the worktree,
Git directory, common directory, and exact `GIT_INDEX_FILE` into the container,
clears inherited host Git environment, and calls the installed launcher's
`--exec-only` mode. The container supplies Pre-commit, which isolates staged
changes from unstaged edits before running `.pre-commit-config.yaml` checks.
The configuration delegates to fixture-free `just check-local`; failure blocks
the commit. It never installs fixtures or runs conformance.

Start the current worktree's container explicitly with `./dev true` or a normal
`./dev just ...` command before `git commit`. Hook execution never starts,
rebuilds, or recreates containers. A missing, stopped, or stale container is an
error with a manual recovery instruction, not a lifecycle operation during a
commit. Do not bypass hooks unless the user explicitly requests it.

`./dev just pre-commit` runs the same checks manually against all tracked files.
`./dev just tools-test` checks installation, worktree/index mapping, and dispatch
without Docker using temporary repositories and fake container commands. The
helper entry point and tests are described in `tools/repo/README.md`.

## Proprietary Environments

A private setup lives in ignored `.devcontainer.local/` and is selected through
`ONDAS_DEV_CONFIG`. Prefer the public Dockerfile plus explicit runtime mounts,
network settings, and environment. Mount vendor installations and credential
files read-only unless a specific operation requires writes.

Use a local Dockerfile only for requirements that runtime configuration cannot
meet, such as extra packages or linker preparation. Preserve the public workspace
layout, user, and command-entry model. Small duplication in profile JSON is
preferable to a profile generator or configuration-merging framework.

`just.local` is the reserved ignored location for optional private recipes when
an optional import is enabled in the public `justfile`. It adds commands, not
alternate meanings for public recipes. In particular, `just ci` must have the
same meaning regardless of installed vendor tools. Local recipes choose SDK
versions and environment explicitly; builds against different SDKs must use
separate `CARGO_TARGET_DIR` values when their outputs depend on headers or ABI.

The launcher must not discover EDA installations, select versions, configure
licenses, or branch on backend names. These are local profile/recipe concerns,
not public infrastructure. Private CI topology is outside this repository's
public contract.

## Fixtures and Privacy

The launcher mounts an already materialized fixture root. It does not download
providers, check catalog versions, generate artifacts, update hashes, or change
locks. Validation belongs to fixture tooling inside the container; see
[fixtures.md](fixtures.md).

Keep `.env`, `just.local`, `.devcontainer.local/`, `tools.local/`, materialized
fixtures, vendor installations, credentials, private sidecars, and infrastructure
details out of both Git and Docker build contexts. An ignored file is not
automatically excluded from a Docker context. Public runs must work without
private credentials or a license network.

Use ignored root `tmp/` for scratch, logs, and disposable outputs; do not erase
other users' or agents' files there. Tracked temporary work belongs only in
`docs/wip/yymmdd-slug/`. Promote durable conclusions and remove task directories
before merging into `master`, retaining `docs/wip/AGENTS.md`. This is a textual
contribution rule, not a hook, script, or CI gate.

## Recipe and Tool Boundaries

Keep recipe names semantic and stable. The public CI recipe is a reproducible
quality gate shared by local development and GitHub Actions, not an environment-
dependent dispatcher. Private suites are explicitly requested commands. Gate
coverage grows with actual checks; do not add empty recipes to suggest coverage.
Benchmark compilation and smoke execution may be checks, but wall-clock timings
are not gates.

Keep short wrappers in `justfile`. A nontrivial public helper belongs in
`tools/<name>/` with a README, explicit entry point, tests, and a normal `just`
recipe. Its README explains purpose, inputs, outputs and modified files,
dependencies, usage, tests, and exclusions. Use a testable supported language for
parsing, checksums, filesystem validation, and subprocess orchestration; reserve
shell for thin wrappers.

Shared fixture validation, binding generation, environment checks, and release
helpers are automation. Waveform decoding, hierarchy, values, projections, and
queries belong in the library, not `tools/`. Keep genuinely private helpers under
ignored `tools.local/`. Do not create a common tool framework before meaningful
shared logic exists. Provider materialization can be a separate tool but must
never become an implicit side effect of a query or fixture test.

## CI and Rust Versions

Public GitHub Actions uses only the public devcontainer and the same `just ci`
as a contributor. It neither reads local `.env` nor selects a private profile.
It runs on pushes to `main` and on pull requests, not other branch pushes.
BuildKit's GitHub Actions layer cache reuses the public Dockerfile build; the
runtime config retains the public container settings and uses that locally loaded
image. No image registry publication credentials are needed.

The fixture checkout/payload cache is keyed by OS and the complete lock-file hash.
CI sets container-side `ONDAS_FIXTURES`, runs `just fixtures-install` even after a
cache hit, then `just ci && just msrv`. The installer clones the exact `v<version>`
tag from `kleverhq/ondas-fixtures`, checks checkout/catalog identity and invokes
the provider installer without `--ignore-missing`. Existing payloads are size/hash
verified; absent assets or corrupt downloads fail. No latest-tag fallback or
cross-version cache restore is used. GitHub's read-only token supplies API access;
fork PRs require no private secrets.

Locally, install explicitly using the same target before running full CI. Neither
`just ci` nor `just check-local` downloads fixtures implicitly. An existing checkout
at another revision or with tracked edits is rejected, never reset: choose another
fixture root or deliberately manage that checkout yourself. Missing required
inputs are errors, not conditional skips.

`rust-toolchain.toml` pins the development toolchain; `Cargo.toml`'s `rust-version`
is the minimum supported Rust version. The separate `just msrv` check uses that
package field. Keep container toolchain installation consistent with both.

Choose the MSRV from the supported feature dependency graph, language/standard
library requirements, and any justified project floor. Do not derive it blindly
from current stable or an old dependency snapshot. Check proprietary
configurations at the same MSRV in their explicit environment. Raise the floor
only deliberately, record the change in package metadata and the release
changelog, and do not ship an MSRV increase in a patch release.

## Public Documentation

Rustdoc in `src/` owns API contracts and usable examples; `docs/` owns developer
concepts and policies. Build documentation with warnings denied. Cargo's
`package.metadata.docs.rs` selects the feature/target configuration, and
`just docs` reproduces it locally. docs.rs builds automatically after crates.io
publication; no separate documentation site or publish job is needed.

Prefer public all-features compilation and documentation without vendor
installations, with runtime absence reported as a backend-availability error.
Cargo still builds dependencies and runs their build scripts during `cargo doc`:
a documentation build does not magically avoid proprietary SDK discovery.

If necessary, a small `DOCS_RS`/`cfg(docsrs)` path may bypass discovery while
preserving the public API. Do not build a parallel fake reader for documentation.
If a dependency cannot support that path cleanly, select a reliable explicit
open-source feature set instead. Documentation success does not prove private
linking, licensing, or runtime compatibility.

## Release Contract

A library release consists of a crates.io package, a `vX.Y.Z` Git tag, and a
GitHub Release. It does not need a binary matrix, release assets, or GitHub Pages.
Release preparation updates the package version, changed lockfile content, and
the corresponding changelog section.

A release check must not publish. Its scope is the public CI gate, MSRV check,
documentation build, version/changelog consistency, package-content inspection
(`cargo package --list`), and `cargo publish --dry-run`. Executable recipe and
workflow availability remain defined by `justfile` and `.github/workflows/`.

Publish from a version tag after merge, verify the release, publish the crate,
then create the GitHub Release from the version's changelog section. Prefer
GitHub Actions OIDC Trusted Publishing once configured for the crate over a
long-lived registry token. Do not create a successful GitHub Release before
package publication succeeds. docs.rs queues its build asynchronously; the
release does not wait for it.
