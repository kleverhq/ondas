# Fixture integration and oracle evidence

The `fixtures/` Git submodule owns the sidecar and sparse-oracle contract in
[its unified fixture schema](https://github.com/kleverhq/ondas-fixtures/blob/d48ab29339d80c653d0bf0196e282ee6fb431ffb/schemas/fixture.schema.json).
The oracle definition is under `$defs.oracle`. Sidecar and nonempty oracle
`schema: 1` markers remain independent of the selected Git commit. Ondas keeps no
schema copy. This document owns local integration, runner coverage and the
interpretation of evidence through the Ondas API. See [testing](testing.md) for
strategy and [the fixture README](https://github.com/kleverhq/ondas-fixtures/blob/d48ab29339d80c653d0bf0196e282ee6fb431ffb/README.md)
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
For SHM directories, it verifies every DSN/TRN component against `artifact.files`.
The directory must contain exactly the listed components. The artifact size is
their total size. Its SHA-256 covers the sorted canonical JSON manifest, with
ASCII escapes, rather than concatenated waveform bytes.

The fixture repository validates metadata, including provenance, tags and
relations. These fields supply no reader observations. An absent oracle and `{}`
supply no conformance evidence. The runner still verifies the required artifact,
then reports it as skipped. A required pool must execute at least one case with evidence.
The GHW pool requires a nonempty profile for every artifact and rejects any
no-oracle skip.

Conformance executes FST, VCD and GHW in file and bytes modes. The explicit
`fsdb-lib` and `wlf-lib` suites execute files with their vendor runtimes. The
explicit `shm-lib` suite executes complete SHM directories with the selected
Xcelium reader.
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
through which it can be looked up. Variables with the same ID describe aliases
of one source history. The runner checks handle equality when the reader supplies
alias identity. Otherwise, it checks each separate handle against that history's
observations. Different IDs must have distinct whole handles.

### Current runner coverage

`tests/conformance.rs` executes the assertion forms used by the pinned pools.
It is not a general JSON Schema validator or a complete oracle-language executor.
Producer validation checks the full schema. The Ondas runner checks its executable
profile and semantic evidence before opening selected waveforms.

Nonempty successful pool oracles currently need explicit `metadata`, `hierarchy`
and `signals` sections, with each signal ID referenced by a variable. An empty
`signals` section asserts only the supplied metadata and hierarchy evidence.
Supported scope assertions are `path`, `kind`, `packing` and `definition_name`. Supported
variable assertions are `path`, `kind`, `direction`, `range`, `type_name`,
`is_constant`, `enumeration` and `signal`. Those fields are checked during execution. Supported
signal encodings are bits, real, string and event, with successful samples/windows.
Opening-error oracles currently cover `malformed` only.

The schema's additional sparse forms, spelling assertions,
unsupported encodings, `unknown-format` opening assertions and sample/window
errors are not executable by this runner. Unsupported assertion forms are rejected
rather than silently counted as passing observations. Local API tests cover
contracts beyond this profile. See [coverage](api-coverage.md). Passing a pool is
not complete schema coverage.

### Values, samples and windows

Bit values retain all nine logic states and compare exactly at the declared width.
Supported ASCII strings compare exactly. Non-NaN reals compare their binary64 representations,
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

### Supported-value limits

The runner retains the installed version-1 sidecars unchanged. Some older string
observations contain non-ASCII text. Those observations now prove a typed
`Error::NonAsciiString` rejection. The test-side derivation retains explicit
samples and complete window prefixes before the first proven rejection bound.
Point, owned history and no-match query checks assert the typed failure by that
bound. Other signals keep their observations. VCD validates all values at open,
so a proven non-ASCII record makes opening fail with that typed error. These are
executed rejection checks, not skips or successful string comparisons. Sparse
evidence can bound a failure without proving the exact first offending tick.

The native GHDL reader uses conventional 1 fs in its version-1 observations. GHW
stores no resolution. The runner checks that known convention separately and
expects absent Ondas timescale while comparing raw ticks unchanged. It does not
treat the conventional number as evidence of a physical duration.

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

FSDB cases can be skipped after input validation when a failed SDK open reports
a newer file version than the Reader API. The runner prints the reason and counts
these cases separately. Unexpected opening errors and observation mismatches fail.
A required pool still needs at least one executed case with evidence. Keep SDK
selection in ignored local configuration, not in fixture sidecars.

The matrix is `fixture × explicitly selected compatible backend × supported input
mode`. All waveform operations go through the public Ondas API. Sidecars describe
waveforms, not reader implementations. Automatic reader selection is tested
separately so priority changes cannot remove an adapter from coverage. Reusable
checks derive samples, traces, scans, projections and composed queries from
independent oracle evidence. Concrete coverage belongs in [testing](testing.md).

Some pinned FSDB and WLF oracles contain Verilog SDK identifier spelling in
component paths. The runner maps only the audited fixture inputs to logical
components before comparison. The mapping is explicit in `logical_name_oracle`
in `tests/conformance.rs` and does not call production name adapters. Other
fixture paths remain exact. Raw sidecars, artifact hashes and observations stay
unchanged. Directed name tests use literal expected components through the public
API and check escape flags, diagnostic spelling, lookup and parent/child links.

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
./dev just ci-wlf
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

Local fixture branches can add unpublished recordings for development. Materialize
their ignored payloads before running the installer or tests. Independent readers
supply expected observations; Ondas output does not. Record local source and
fixture commits, and distinguish these inputs from the released pin. Keep the
parent Git entry at its released revision until publication is explicitly
requested. Local success does not establish reproducibility from a remote clone.
Do not run `git submodule update` over a fixture development branch.
