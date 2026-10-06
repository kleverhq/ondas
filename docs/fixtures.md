# Fixture integration and oracle evidence

The fixture provider owns the catalog, sidecar and sparse-oracle schemas. For the
release selected by `fixtures.lock.toml`, the authoritative definitions are
[the catalog schema](https://github.com/kleverhq/ondas-fixtures/blob/v6.0.0/schemas/catalog.schema.json),
[the sidecar schema](https://github.com/kleverhq/ondas-fixtures/blob/v6.0.0/schemas/fixture.schema.json)
and [the oracle schema](https://github.com/kleverhq/ondas-fixtures/blob/v6.0.0/schemas/oracle.schema.json).
All three retain schema version 1; provider version 6.0.0 is a separate version.
Ondas keeps no schema copy. This document owns local integration, runner coverage
and the interpretation of oracle evidence through the Ondas API. See
[testing](testing.md) for test strategy and
[the provider README](https://github.com/kleverhq/ondas-fixtures/blob/v6.0.0/README.md)
for delivery and producer commands.

## Catalog root and paths

Fixture-driven suites consume an already materialized directory through the
`ONDAS_FIXTURES` process environment variable. Only `./dev` loads the host `.env`;
the runner consumes the resulting environment and does not load `.env` itself.
CI may export the variable directly. For example:

```dotenv
ONDAS_FIXTURES=/home/user/.cache/ondas/fixtures
```

The fixture root can be external or the ignored repository-root `fixtures/`
directory. Materialized artifacts and local `.env` stay outside tracked source.
A symlinked external provider also needs its target mounted inside the container.
Self-contained tests do not read `ONDAS_FIXTURES`; explicitly requested fixture
suites require it and fail if it is absent.

The provider uses format directories:

```text
$ONDAS_FIXTURES/
└── kleverhq.ondas-fixtures/
    ├── catalog.json
    ├── schemas/
    └── <format>/
        └── <fixture-name>/
            ├── fixture.json
            └── waveform.<format>
```

Fixture references are paths such as `fst/fst0041-counter`, relative to
the provider root. The format directory must agree with `artifact.format`.
`catalog.json.provider` must match the provider directory and selected lock
entry. Source URLs describe provenance, not provider identity.

The runner discovers immediate fixture directories inside the selected format
directory, rather than recursively searching the provider. Paths, including
symlinks, must resolve inside the provider and fixture boundaries.

## Provider versions and installation

`fixtures.lock.toml` selects the provider version:

```toml
[providers]
"kleverhq.ondas-fixtures" = "6.0.0"
```

The selected version must exactly equal `catalog.json.version`. The provider's
catalog schema requires SemVer; Ondas compares the locked string without resolving
version ranges. The lock contains versions only: no URLs, asset names, credentials,
release checksums, materialization commands or fixture lists. Delivery is separate
from test semantics.

`./dev just fixtures-install` installs the provider explicitly. It clones
its exact `v<version>` tag when absent, rejects existing checkouts at another
revision or with tracked changes, verifies catalog identity/version, then runs
`just install` in the provider checkout. The provider verifies payload sizes and
hashes, including cached data. There is no implicit installation during tests,
no latest-version fallback and no resetting an existing checkout.

Tests and benchmarks select `kleverhq.ondas-fixtures` through the repository
lock. `ONDAS_FIXTURES` selects its parent directory. See [local snapshots](#local-snapshots)
for development inputs.

## Sidecars and supported formats

Sidecar structure follows the provider's schema, including its top-level field
restrictions. `provenance.license` is required; use the producer's explicit
`unknown` value when a license is not established. Imported and converted data
retain their source, and converted data retain a nonempty string or object in
`provenance.transform`. Never automatically assign the provider repository's
license to an imported waveform.

Tags are unique nonempty strings, with no Ondas-specific whitelist. They describe
the fixture and may aid external selection; the current pool runner selects by
artifact format. Neither tags nor relations supply oracle observations. An absent
oracle and `{}` both supply no conformance evidence. An installed artifact without
observations is validated and reported as skipped; a required pool must execute
at least one case with evidence.

For the selected single-file formats, `artifact.file` is `waveform.<format>` and
must resolve to a regular file inside the fixture directory. Byte size and SHA-256
bind the sidecar to the exact artifact. Declared and detected format are checked
independently where applicable. Recipes, simulators, logs, extraction commands
and temporary files are outside runtime fixture data. Tests never execute sidecar
commands.

The conformance suite executes FST and VCD in file and bytes modes. The
explicit `fsdb-lib` suite executes FSDB in file mode with the real vendor runtime.
The provider also describes GHW, WLF and SHM artifacts, including multi-file SHM
directories; their presence does not supply Ondas readers or conformance coverage.
This integration checks the selected reader formats only.

## Sparse oracle semantics

### Scope, version and omission

A version-1 oracle is a sparse set of expected observations, independent of the
backend. It is ordered-observation evidence, not a serialization of current
normalized query output. The runner derives current expectations without
rewriting installed sidecars or silently upgrading the oracle schema.

An omitted field makes no assertion. A permitted `null` asserts absence, except
for the unknown-timestamp meaning of `changed_at`. Empty hierarchy lists do not
assert that no other objects exist. Unlisted aliases, signals and ticks are not
negative assertions. Schema structure and permitted fields come from the
provider's oracle schema, not from this document.

Ticks and occurrence counts use canonical unsigned decimal strings fitting u64;
never convert them to seconds for comparison. Declaration bounds are lossless i64
JSON integers, preserving ascending or descending direction. Paths are exact
component arrays without Unicode normalization. Local oracle signal IDs are not
backend handles: references must resolve, and every listed signal needs a variable
through which it can be looked up. Same-ID variables assert alias identity;
different IDs assert distinct whole signals.

### Current runner coverage

`tests/conformance.rs` executes the assertion forms used by the locked pools.
It is not a general JSON Schema validator or a complete oracle-language executor.
Producer validation checks the full schema; the Ondas runner checks its executable
profile and semantic evidence before opening selected waveforms.

Nonempty successful pool oracles currently need explicit `metadata`, `hierarchy`
and nonempty `signals` sections, with each signal ID referenced by a variable.
Supported scope assertions are `path`, `kind` and `definition_name`; supported
variable assertions are `path`, `kind`, `direction`, `range`, `type_name`,
`is_constant` and `signal`. Those fields are checked during execution. Supported
signal encodings are bits, real, string and event, with successful samples/windows.
Opening-error oracles currently cover `malformed` only.

The schema's additional sparse forms, packing/spelling/enumeration assertions,
unsupported encodings, `unknown-format` opening assertions and sample/window
errors are not executable by this runner. Unsupported assertion forms are rejected
rather than silently counted as passing observations. Local API tests cover
contracts beyond this profile; see [coverage](api-coverage.md). Passing a pool is
not complete schema coverage.

### Values, samples and windows

Bit values retain all nine logic states and compare exactly at the declared width.
Strings compare exactly. Non-NaN reals compare their binary64 representations,
including signed zeros and infinities. Version-1 evidence treats any NaN as equal
to any NaN, so it cannot assert payload, sign or signaling-bit identity. This
evidence limit does not redefine the library's representation identity.

A persistent sample describes the final state after all writes at its tick.
Events have no persistent state: samples assert occurrence counts, including zero,
and windows record unit occurrences. `changed_at` omission or `null` supplies no
exact timestamp assertion. A concrete raw timestamp describes establishment under
version-1 observations; it is not automatically a normalized net-change time.
Returned timestamps must still satisfy public bounds.

A successful finite window is inclusive and supplies the complete ordered history
of one signal inside its bounds. Its initial state is strictly before the start,
or absent; events always have no initial state. Same-tick records of one signal
retain their order, and event occurrences retain multiplicity. Redundant persistent
writes may be absent, but distinct intermediate values must remain. An empty
reversed window supplies no initial state or changes. Overlapping windows and
samples must agree; evidence is not a set of alternative accepted answers.

### Normalized expectations

The independent test-side derivation groups each complete window's records by
tick. A persistent slot uses the final value and emits it only when it differs
from the entering value, or first establishes a state. HDL unknown is a value,
not missing history. Unit event records become one positive observed count per
tick; absent ticks have zero events. Aggregation neither restores events omitted
by a producer nor establishes ordering within a tick. Project each source value
before deriving slice changes so activity in discarded bits and net-equal slice
excursions do not create changes.

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
prove a new net-change time after a known entering state; net-equal excursions
preserve an already proven time. Earlier covering windows can establish a window's
entering timestamp and carry that proof through later complete windows, but never
across an uncovered interval. First establishment without known entering state
and initial states without prior coverage do not supply an exact time. A
NaN-to-NaN observation cannot prove representation equality; while holding NaN,
omitted payload-only writes can also prevent an exact timestamp inference. A
definite transition from a non-NaN value into NaN proves that tick, not necessarily
the establishment time at later ticks. Public timestamp bounds remain checked
when an exact time is unavailable. A proven timestamp is compared whenever the
reader returns a known timestamp, retaining the public allowance for unknown time.

The runner checks points at raw observation ticks and their neighbors even when
an excursion disappears from normalized traces. Preflight validation compares
version-1 ordered histories and typed value, missing-state and event-count
evidence before waveform execution. Overlapping evidence must agree; it is not a
set of alternative accepted answers. Derived counts and timestamps are ephemeral
expectations, not new version-1 serialized fields. The validator still accepts `{"event":true}` as a unit value and rejects aggregate values in
version-1 windows. New serialized assertions would require an explicit version.
Version 1 also supplies no signedness or logic-domain fields; do not infer them
from names or observed bits. Independent declaration tests cover those contracts.

Overlapping windows and samples of a signal must agree. `time_span` does not forbid
queries before or after the recorded range: after EOF, a persistent sample retains
the last value and an event sample has zero occurrences.

## Validation and execution

Before opening any waveform, the runner validates the provider's catalog
identity and exact locked version, discovered sidecars, path containment, artifact
sizes/hashes and supported oracle evidence. It checks tag uniqueness, provenance,
canonical ticks, declaration bounds, references, encoding/value compatibility,
window ordering and overlapping observations. Invalid installed data fails before
conformance. Artifacts are hashed once during preflight, not for every query.
Producer schema validation remains necessary; this runner does not certify every
schema rule or assertion form.

Missing providers or artifacts, unavailable readers, invalid data and empty
selections fail. A deliberately malformed waveform is different: a valid sidecar
may assert the waveform error that the API should return.

The matrix is `fixture × explicitly selected compatible backend × supported input
mode`. All waveform operations go through the public Ondas API. Sidecars describe
waveforms, not reader implementations. Automatic reader selection is tested
separately so priority changes cannot remove an adapter from coverage. Reusable
checks derive samples, traces, scans, projections and composed queries from
independent oracle evidence; concrete coverage belongs in [testing](testing.md).

## Local snapshots

A local snapshot can replace the installed provider for development when its
catalog identity and version match `fixtures.lock.toml`. Keep the complete
snapshot under ignored `tmp/` and select its parent with `ONDAS_FIXTURES`. Use the
same root for conformance and measurements. The tagged installer does not apply
to an unpublished version. A locally edited lock is integration work, not proof
that contributors or CI can install that version. See
[benchmarking](benchmarking.md#local-fixture-snapshots) for commands.

The runner reads local data only. It does not run simulators, download or
rebuild waveforms, update hashes or publish catalogs. The fixture producer owns
schemas, generation and delivery.
