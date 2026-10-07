# Fixture integration and oracle evidence

The `fixtures/` Git submodule owns the sidecar and sparse-oracle contract in
[its unified fixture schema](https://github.com/kleverhq/ondas-fixtures/blob/aadb6d00597265fdef1a8227825465dbffecee43/schemas/fixture.schema.json).
The oracle definition is under `$defs.oracle`. Sidecar and nonempty oracle
`schema: 1` markers remain independent of the selected Git commit. Ondas keeps no
schema copy. This document owns local integration, runner coverage and the
interpretation of evidence through the Ondas API. See [testing](testing.md) for
strategy and [the fixture README](https://github.com/kleverhq/ondas-fixtures/blob/aadb6d00597265fdef1a8227825465dbffecee43/README.md)
for generation, validation and artifact delivery.

## Revision and installation

The parent repository records one exact fixture commit in its `fixtures/` Git
entry. Initialize that checkout on the host, then explicitly install waveform
payloads through the development container:

```sh
git submodule update --init fixtures
./dev just fixtures-install
```

The installation recipe invokes the submodule's `just install`. That installer
finds release assets by the sidecars' recorded hashes and verifies their sizes
and SHA-256, including cached files. Release tags identify delivery assets. The Git
entry selects the corpus. Quality gates never initialize, download or generate
fixtures implicitly. Default tests need no initialized submodule or payloads.

Tests and benchmarks resolve `fixtures/` from the repository root, independently
of the process working directory. Fixture references such as `fst/fst0041-counter`
use this layout:

```text
fixtures/
├── schemas/fixture.schema.json
└── <format>/<fixture-name>/
    ├── fixture.json
    └── waveform.<format>
```

Discovery visits immediate fixture directories within the selected format. The
format directory must agree with `artifact.format`. Paths, including symlinks,
must resolve inside the fixture root and each fixture's boundaries. Selected
single-file artifacts must be regular files. The shared loader in
`tests/support/fixtures.rs` verifies artifact size and SHA-256 before execution.

The fixture repository validates metadata, including provenance, tags and
relations. These fields supply no reader observations. An absent oracle and `{}`
supply no conformance evidence. The runner still verifies the required artifact,
then reports it as skipped. A required pool must execute at least one case with evidence.

Conformance executes FST and VCD in file and bytes modes. The explicit `fsdb-lib`
suite executes FSDB in file mode with the vendor runtime. GHW, WLF and SHM
sidecars in the submodule do not supply Ondas readers or conformance coverage.
Fixtures are excluded from the crate package and Docker build context. Downloaded
payloads are ignored by the submodule's Git rules.

## Sparse oracle semantics

### Scope, version and omission

A version-1 oracle is a sparse set of expected observations, independent of the
backend. It records ordered observations. It does not serialize current normalized
query output. The runner derives current expectations without
rewriting installed sidecars or silently upgrading the oracle schema.

An omitted field makes no assertion. A permitted `null` asserts absence, except
for the unknown-timestamp meaning of `changed_at`. Empty hierarchy lists do not
assert that no other objects exist. Unlisted aliases, signals and ticks are not
negative assertions. Schema structure and permitted fields come from the
fixture schema, not from this document.

Ticks and occurrence counts use canonical unsigned decimal strings that fit u64.
Never convert them to seconds for comparison. Declaration bounds are lossless i64
JSON integers. They preserve ascending or descending direction. Paths are exact
component arrays without Unicode normalization. Local oracle signal IDs are not
backend handles. References must resolve. Every listed signal needs a variable
through which it can be looked up. Variables with the same ID assert alias identity.
Different IDs assert distinct whole signals.

### Current runner coverage

`tests/conformance.rs` executes the assertion forms used by the pinned pools.
It is not a general JSON Schema validator or a complete oracle-language executor.
Producer validation checks the full schema. The Ondas runner checks its executable
profile and semantic evidence before opening selected waveforms.

Nonempty successful pool oracles currently need explicit `metadata`, `hierarchy`
and nonempty `signals` sections, with each signal ID referenced by a variable.
Supported scope assertions are `path`, `kind` and `definition_name`. Supported
variable assertions are `path`, `kind`, `direction`, `range`, `type_name`,
`is_constant` and `signal`. Those fields are checked during execution. Supported
signal encodings are bits, real, string and event, with successful samples/windows.
Opening-error oracles currently cover `malformed` only.

The schema's additional sparse forms, packing/spelling/enumeration assertions,
unsupported encodings, `unknown-format` opening assertions and sample/window
errors are not executable by this runner. Unsupported assertion forms are rejected
rather than silently counted as passing observations. Local API tests cover
contracts beyond this profile. See [coverage](api-coverage.md). Passing a pool is
not complete schema coverage.

### Values, samples and windows

Bit values retain all nine logic states and compare exactly at the declared width.
Strings compare exactly. Non-NaN reals compare their binary64 representations,
including signed zeros and infinities. Version-1 evidence treats any NaN as equal
to any NaN, so it cannot assert payload, sign or signaling-bit identity. This
evidence limit does not redefine the library's representation identity.

A persistent sample describes the final state after all writes at its tick.
Events have no persistent state. Samples assert occurrence counts, including zero,
and windows record unit occurrences. `changed_at` omission or `null` supplies no
exact timestamp assertion. A concrete raw timestamp describes establishment under
version-1 observations. It is not automatically a normalized net-change time.
Returned timestamps must still satisfy public bounds.

A successful finite window is inclusive and supplies the complete ordered history
of one signal inside its bounds. Its initial state is strictly before the start,
or absent. Events always have no initial state. Same-tick records of one signal
retain their order, and event occurrences retain multiplicity. Redundant persistent
writes may be absent, but distinct intermediate values must remain. An empty
reversed window supplies no initial state or changes. Overlapping windows and
samples must agree. Evidence is not a set of alternative accepted answers.

### Normalized expectations

The independent test-side derivation groups each complete window's records by
tick. A persistent slot uses the final value and emits it only when it differs
from the entering value, or first establishes a state. HDL unknown is a value,
not missing history. Unit event records become one positive observed count per
tick. Absent ticks have zero events. Aggregation neither restores events omitted
by a producer nor establishes ordering within a tick. Project each source value
before deriving slice changes. Activity in discarded bits must not create slice
changes. If a slice changes and returns to its entering value within one tick,
it must not produce a net change.

Version-1 NaN equivalence is a limit on expected-value evidence. The comparison
ignores additional actual NaN-payload transitions that this evidence cannot
resolve. Independently, actual output must still have at most one record per
slot/tick and must not repeat an exactly identical persistent representation.
Signed zeros, non-NaN real patterns, strings and all nine logic states retain their
justified exact assertions. Stronger NaN-payload identity is tested with explicitly
authored local histories, not inferred from legacy sidecars.

A raw `changed_at` may describe a write or intermediate excursion removed by
normalization. Isolated samples continue to assert values, presence and counts,
but cannot alone establish a normalized timestamp. Complete covering windows can
prove a new net-change time after a known entering state. Changes that return to
the entering value within one tick preserve an already proven time.
Earlier covering windows can establish a window's
entering timestamp and carry that proof through later complete windows, but never
across an uncovered interval. First establishment without known entering state
and initial states without prior coverage do not supply an exact time. A
NaN-to-NaN observation cannot prove representation equality. While the value is NaN,
omitted payload-only writes can also prevent an exact timestamp inference. A
definite transition from a non-NaN value into NaN proves that tick, not necessarily
the establishment time at later ticks. Public timestamp bounds remain checked
when an exact time is unavailable. A proven timestamp is compared whenever the
reader returns a known timestamp, retaining the public allowance for unknown time.

The runner checks points at raw observation ticks and their neighbors even when
an excursion disappears from normalized traces. Preflight validation compares
version-1 ordered histories and typed value, missing-state and event-count
evidence before waveform execution. Overlapping evidence must agree.
Derived counts and timestamps are temporary expectations. They do not add
serialized fields to version 1. The runner still accepts `{"event":true}` as a
unit value and rejects aggregate values in
version-1 windows. New serialized assertions would require an explicit version.
Version 1 also supplies no signedness or logic-domain fields. Do not infer them
from names or observed bits. Independent declaration tests cover those contracts.

Overlapping windows and samples of a signal must agree. `time_span` does not forbid
queries before or after the recorded range. After EOF, a persistent sample retains
the last value and an event sample has zero occurrences.

## Validation and execution

Before opening any waveform, the runner checks discovered sidecars, schema-version
compatibility, path containment, artifact sizes/hashes, and supported oracle
evidence. Its executable profile checks signal references, representable ticks
and encodings, window ordering, and overlapping observations. It rejects assertion
forms it cannot execute. Metadata validation belongs to the fixture repository;
Ondas does not implement or run a second JSON Schema validator. Artifacts are
hashed once during preflight, rather than for every query.

An uninitialized submodule, missing artifacts, unavailable readers, unsupported
assertions, inconsistent evidence, and empty selections fail. A deliberately
malformed waveform is different: its sidecar can assert the API's opening error.

The matrix is `fixture × explicitly selected compatible backend × supported input
mode`. All waveform operations go through the public Ondas API. Sidecars describe
waveforms, not reader implementations. Automatic reader selection is tested
separately so priority changes cannot remove an adapter from coverage. Reusable
checks derive samples, traces, scans, projections and composed queries from
independent oracle evidence. Concrete coverage belongs in [testing](testing.md).

## Updating the corpus

Select a new fixture commit deliberately on the host, install its payloads, and
run the relevant conformance and benchmark checks before committing the updated
Git entry:

```sh
git -C fixtures fetch origin
git -C fixtures checkout --detach <commit>
./dev just fixtures-install
./dev just conformance
./dev just bench-smoke
# With the SDK profile selected:
./dev just ci-fsdb
git add fixtures
```

Commit any required consumer changes and update pinned schema links together
with the Git entry. Then run `./dev just ci` on the clean commit. After switching
parent branches, use
`git submodule update --init fixtures` to restore their selected revision. Do not
use a moving branch as an automatic input to tests or CI.

Local fixture edits can support experiments inside the submodule. Validate them
with the fixture repository's tools and use the same checkout for conformance
and measurements. Shared results require an available fixture commit and matching
parent Git entry. See [benchmarking](benchmarking.md#fixture-revisions).

The runner reads installed data only. It never executes sidecar commands,
simulates, repairs artifacts, updates hashes or publishes fixture releases.
