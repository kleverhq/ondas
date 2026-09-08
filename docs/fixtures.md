# Fixture catalog and oracle contract

This document defines the local filesystem contract for waveform fixtures, provider
versions, sidecars, and backend-neutral expected observations. The normative
structural schema for the `oracle` value is [oracle.schema.json](oracle.schema.json)
(JSON Schema Draft 2020-12); its additional semantic constraints are defined here.
That schema does not describe the provider catalog or the whole sidecar envelope.
See [Testing](testing.md) for test strategy.

## Catalog root and identity

Fixture-driven suites consume an already materialized directory through the
`ONDAS_FIXTURES` process environment variable. Only `./dev` loads the host `.env`;
the runner consumes the resulting environment and does not load `.env` itself.
CI may export the variable directly. For example:

```dotenv
ONDAS_FIXTURES=/home/user/.cache/ondas/fixtures
```

The fixture root can be external or the ignored repository-root `fixtures/`
directory. Materialized artifacts and local `.env` must stay outside tracked source.
A symlinked external provider also needs its target mounted inside the container. Self-contained unit tests do not read
`ONDAS_FIXTURES`; an explicitly requested fixture suite requires it and fails if
it is absent.

```text
$ONDAS_FIXTURES/
└── <provider>/
    ├── catalog.json
    └── <fixture-name>/
        ├── fixture.json
        └── waveform.<format>
```

There may be multiple providers and multiple fixtures per provider. Fixture
identity is exactly `<provider>/<fixture-name>`. Both parts are directory names
valid as a single filesystem path component. Fixture names are unique within a
provider. The provider is an opaque, stable namespace; deriving it from a
repository name, such as `kleverhq.ondas-fixtures`, is recommended for a
repository-backed catalog. Source URLs are neither part of identity nor required
by the runner.

Each fixture directory contains exactly one waveform artifact named
`waveform.<format>` and one sidecar named `fixture.json`. Materialization sources,
simulator projects, recipes, logs, and temporary files are not part of the runtime
catalog. Identity is independent of storage or delivery method.

## Provider metadata and version locking

Each provider root contains `catalog.json`, with this minimal shape:

```json
{
  "schema": 1,
  "provider": "kleverhq.ondas-fixtures",
  "version": "1.0.0"
}
```

- `schema` identifies the catalog metadata schema, here `1`.
- `provider` must exactly match both the directory name and the selected lock entry.
- `version` is an opaque string, not necessarily a number or semantic version. It
  changes whenever published catalog contents change in a way significant to tests.

The catalog need not enumerate fixtures. Discovery uses immediate child
directories containing `fixture.json`.

The repository lock-file contract is named `fixtures.lock.toml`. It contains only
required provider versions:

```toml
[providers]
"kleverhq.ondas-fixtures" = "1.0.0"
```

Every version must exactly equal the provider's `catalog.json.version`. The lock
contains no download URLs, archive names, storage credentials, release tags,
delivery-archive checksums, materialization commands, or individual fixture list.
Those belong to delivery, not test semantics.

A public fixture run uses the repository lock by default. A controlled environment
may select a different lock with the same structure; private-lock selection and
composition mechanisms are outside this contract. Extra provider directories under
`ONDAS_FIXTURES` are allowed. A run validates and uses the providers in its selected
lock, not every provider present on disk.

## Sidecar envelope

Each fixture has one schema-1 `fixture.json`. Its directory supplies the fixture
name; no duplicate name field is required. A minimal envelope looks like this
(the size and checksum shown are illustrative):

```json
{
  "schema": 1,
  "artifact": {
    "file": "waveform.fst",
    "format": "fst",
    "size": 18432,
    "sha256": "..."
  },
  "provenance": {"kind": "authored"},
  "tags": [],
  "oracle": {}
}
```

Human-readable fields such as `description` are permitted outside the oracle.
They carry no test semantics and must not contain executable materialization
commands required by the runner. The oracle's prohibition on unknown fields does
not define an equivalent closed schema for the envelope.

### Artifact

`artifact` is required:

| Field | Contract |
|---|---|
| `file` | Relative filename within the fixture directory; normative name `waveform.<format>`. |
| `format` | Expected waveform format. |
| `size` | Expected artifact size in bytes. |
| `sha256` | SHA-256 checksum binding the sidecar to the exact artifact contents. |

The path must resolve to a regular file inside the fixture directory. Absolute
paths and directory escapes are invalid. The extension is informational: where
applicable, declared format and detected format are checked independently.

### Provenance

`provenance` is required for every fixture:

| `kind` | Meaning | Required additional fields |
|---|---|---|
| `authored` | Own artifact, handwritten or generated from own sources. | None. |
| `imported` | Artifact from an external source. | `source`. |
| `converted` | Converted artifact. | `source`, `transform`. |

`source` is a stable URL or source fixture identity. `transform` briefly describes
the transformation and tool used, for example `vcd2fst 3.3.126`. Optional `license`
preserves the known source license; omission means the license has not been
established. Never automatically assign a provider repository's license to an
imported artifact. For a minimized or modified external case, choose the
appropriate kind and describe changes in `transform` where useful.

Public corpora follow provenance-first sourcing: authored, found, and converted
artifacts from publicly accessible sources are acceptable when their origin is
preserved. Clearly confidential, closed, or redistribution-prohibited material
must not enter a public provider. A justified rights-holder request or a content
problem may require removing a fixture from subsequent provider versions.
Removal of already published artifacts is a provider-storage concern, not a
runner responsibility.

### Tags

Optional `tags` is an array of unique strings from this exact whitelist; `[]` is
valid. Tags aid selection but neither replace an oracle nor prove a property.

| Tag | Meaning |
|---|---|
| `conformance` | Nonempty oracle for positive conformance checks. |
| `metadata` | Oracle checks stable metadata. |
| `hierarchy` | Oracle checks scopes or declarations. |
| `paths` | Special names and explicit spelling checks. |
| `aliases` | Multiple declarations checked as one underlying signal. |
| `values` | Persistent value samples or changes. |
| `bits` | Bit-vector value checks. |
| `real` | Real value checks. |
| `strings` | String value checks. |
| `events` | Event occurrence checks, including multiplicity. |
| `unknown-states` | Logic states other than `0` and `1`. |
| `ranges` | Original declaration range checks. |
| `projections` | Bit histories permitting slice changes to be checked independently of other bits. |
| `same-time` | Multiple changes or event occurrences of one signal at one tick. |
| `constants` | Constant, parameter, or generic declaration checks. |
| `enumerations` | Enumeration metadata checks. |
| `malformed` | Intentionally damaged artifact; an oracle, when present, describes the error. |
| `large` | Source artifact size at least 50 MiB (52,428,800 bytes). |

Unknown or duplicate tags are validation errors. Adding a tag requires changing
this whitelist; format and reader names are not tags. There is no tag query
language, inheritance, profile system, or tag expression language in this contract.

Empty oracles and empty tags are valid. A catalog containing only empty oracles
can pass catalog validation, but supplies no oracle semantic assertions and cannot
establish semantic conformance. This distinction adds no nonempty-oracle
requirement to the schema.

## Sparse oracle semantics

### Scope, version, and omission

An oracle is a backend-neutral sparse set of expected observations. `{}` means no
reference observations have been supplied. Every nonempty oracle requires
`schema: 1` and `open`; its other sections are `metadata`, `hierarchy`, and
`signals`. The oracle version is independent of the sidecar schema and provider
version. **Unknown fields are forbidden throughout a nonempty oracle**, apart from
the arbitrary local signal IDs used as keys of `signals`.

An omitted field makes no assertion. A permitted `null` asserts absence, except
for the special unknown-timestamp meaning of `changed_at` below. An empty hierarchy
list does not assert that no other items exist. There are no `full` or `complete`
modes, exhaustive hierarchy counts, or global negative assertions about unlisted
objects. Even observations that happen to cover a small waveform entirely remain
a sparse oracle.

The runner executes no commands from the oracle. Recipes, reader versions,
container images, logs, source paths, and oracle-generation provenance belong
outside the runtime sidecar. Artifact provenance remains in the envelope's
`provenance` section. Format comes from `artifact.format`, never a duplicate
oracle field. Performance-specific expectations are outside this contract.

### Common types and references

| Type | Contract |
|---|---|
| Tick or occurrence count | Canonical unsigned decimal **string**, no leading zero except `"0"`, in `0..18446744073709551615` (u64). Schema checks syntax; semantic validation checks the numeric limit. Never floats or ticks converted to seconds. |
| Path | Nonempty array of exact string components. Escaping belongs to spellings, not identity. String comparison is exact, without Unicode normalization. |
| Signal ID | Arbitrary nonempty local string used as a `signals` key; not a backend-local handle. |
| Declaration `msb` / `lsb` | Integer JSON numbers in `-9223372036854775808..9223372036854775807` (i64), read without rounding. Preserve ascending/descending direction; these are not normalized slice bit indices. |

Scope paths and variable paths must each be unique within their respective list.
Ancestors need not be listed. Name and parent path are derived from components,
not duplicated as fields. All non-null `variable.signal` references must resolve.
Every listed signal must have at least one variable reference through which it
can be looked up; signal IDs must be unique.

### Opening and metadata

`open` is exactly one of:

```json
{"result": "ok"}
```

```json
{"result": "error", "kind": "malformed"}
```

```json
{"result": "error", "kind": "unknown-format"}
```

An opening error forbids `metadata`, `hierarchy`, and `signals`. Tool failure,
unavailable licenses or runtime libraries, and cases unsupported by an extractor
are not expected waveform errors.

All metadata fields are optional:

| Field | Expected observation |
|---|---|
| `timescale` | `null`, or `{factor, unit}` with positive u32 factor (`1..4294967295`) and unit `s`, `ms`, `us`, `ns`, `ps`, `fs`, `as`, or `zs`. |
| `time_span` | `null`, or `{first, last}` with `first <= last`: first and last recorded ticks, not first/last changes of a selected signal. |
| `writer`, `date` | String or `null`, extracted from the original waveform, not an intermediate conversion. |
| `comments` | Sparse list of expected strings. Each must occur at least as many times as listed; order is unspecified. |

`source_name`, backend name, and input mode depend on the invocation and selected
implementation, so are not oracle fields. Path/bytes modes belong to the runner's
matrix.

### Hierarchy, declarations, and aliases

`hierarchy.scopes` and `hierarchy.variables` are sparse arrays whose ordering does
not prescribe traversal order. A scope requires `path`; optional fields are
`kind` (string), `definition_name` (string or `null`), `packing` (`packed`,
`unpacked`, `sparse`, `tagged-packed`, or `null`), and `spellings`.

A variable requires `path`; all other fields are optional:

| Field | Contract |
|---|---|
| `kind` | Canonical lower-case declaration kind; unknown vendor kinds retain a namespaced form. |
| `direction` | `unknown`, `implicit`, `input`, `output`, `inout`, `buffer`, or `linkage`. |
| `range` | `{msb, lsb}` or `null`, using the declaration-index rules above. |
| `is_constant` | Boolean declaration property, not an inference from lack of transitions. |
| `type_name` | String or `null`. |
| `enumeration` | `null`, or `{name?, variants}`. Optional `name` is string or `null`; `variants` is the **complete** list of `{encoded, label}` string pairs for that enumeration, with unique `encoded` keys. |
| `signal` | Local signal ID, or `null` when the declaration has no queryable signal. |
| `spellings` | Expected canonical formatting and/or additional accepted paths, as below. |

For either scopes or variables, `spellings` contains optional `canonical` (the
exact expected Ondas formatted path) and `accepted` (unique additional equivalent
string paths). Every spelling must resolve to the same exact component path.

Variables with the same signal reference must yield equal whole signals;
variables with different references must yield different whole signals. An
unlisted alias is not asserted absent. An omitted declaration range must not be
replaced with a guessed `[width-1:0]`.

### Encodings and value equality

`signals` maps local IDs to objects with required `encoding` and optional `samples`
and `windows`. An encoding is `{"kind":"bits","width":N}`, where
`1 <= N <= 4294967295`, or a single `kind` of `real`, `string`, `event`, or
`unsupported`. Unsupported encoding permits no successful value observations.

A value has exactly one key, and its variant must match the signal encoding:

| Example | Representation and comparison |
|---|---|
| `{"bits":"01xz"}` | Exactly `width` characters, MSB to LSB, from lowercase `01xzhuwl-`; no separators. |
| `{"real_bits":"3ff0000000000000"}` | Exactly 16 lowercase hex digits representing an IEEE-754 binary64 bit pattern from high bits to low bits, independently of host endianness. |
| `{"string":"hello\nworld"}` | Exact JSON Unicode string. |
| `{"event":true}` | One event occurrence. |

Finite reals, infinities, and signed zero compare bitwise. **Any NaN equals any
NaN** for oracle comparison; payload and signaling bit are not checked. Readers
must not introduce intermediate decimal rounding.

### Samples and `changed_at`

Each sample has `time` and exactly one result:

| Result | Fields and meaning |
|---|---|
| Persistent value | `value`, optionally `changed_at`. The final state after **all** changes of this signal at `time`. |
| No known persistent state | `missing: true`. |
| Event count | `occurrences`, a u64 string including `"0"`, counting all occurrences exactly at `time`. |
| Observation error | `error: {kind}` under the error rules below. |

Events have no persistent state or `missing`. `changed_at` is allowed only with a
persistent `value`; it is the expected tick at which the observed state was
established, not an unconditional requirement that every backend compute it.

| Oracle `changed_at` | Assertion |
|---|---|
| Omitted | Do not check the returned timestamp. |
| `null` | Exact establishment time is unknown to the oracle. Do not constrain the returned timestamp beyond public invariants. |
| Concrete tick | A known timestamp returned by the backend must equal this tick. An unknown timestamp allowed by the public contract is accepted. |

A concrete sample `changed_at` must be `<= time`. The same unknown-timestamp rules
apply to window initial states, but a concrete `initial.changed_at` must be
**strictly less than** the window start.

Samples are useful for isolated observations without an extracted surrounding
history. Avoid duplicating observations already derivable from windows without a
specific need.

### Exact finite windows

A successful window requires `start`, `end`, `initial`, and `changes`. Both bounds
are finite ticks, and the interval is inclusive. To cover through EOF, use the
established last tick, not an unbounded sentinel. When `start > end`, the window
is empty and must have `initial: null` and `changes: []`.

`initial` describes the known persistent state **strictly before** `start`: either
`null` if none exists, or `{value, changed_at?}`. A known initial timestamp must be
less than `start`. For events, `initial` is always `null`.

`changes` is the **complete** list of `{time, value}` changes of this signal inside
the window, with nondecreasing times. The oracle is sparse globally, but may not
omit changes within a declared successful window.

- No ordering is prescribed between different signals at the same tick.
- Multiple changes of one signal at one tick retain their order; sampling uses
  their final value.
- Redundant writes of persistent values are normalized by comparison with the
  preceding known state and may be absent. Distinct intermediate values must not
  be removed.
- Event occurrences are neither coalesced nor deduplicated, even when time and
  value are identical.
- If no initial state is known at window start, retain the first record that
  establishes the previously unknown state.

The runner derives samples, traces, scans, entering values, and bit projections
from windows. For a slice, check changes of the **projected value**, not every
activity of the base signal. Candidate timestamps must form a strictly increasing
set containing every required change time; additional candidates are permitted.
Early termination, selection order, and repeats are runner checks, not imperative
JSON commands.

Overlapping windows and samples of a signal must agree. `time_span` does not forbid
queries before or after the recorded range: after EOF, a persistent sample retains
the last value and an event sample has zero occurrences.

This sparse example specifies aliases and one exact local history:

```json
{
  "schema": 1,
  "open": {"result": "ok"},
  "metadata": {"timescale": {"factor": 1, "unit": "ns"}},
  "hierarchy": {
    "scopes": [{"path": ["tb"], "kind": "module"}],
    "variables": [
      {"path": ["tb", "data"], "signal": "data"},
      {"path": ["tb", "alias"], "signal": "data"}
    ]
  },
  "signals": {
    "data": {
      "encoding": {"kind": "bits", "width": 4},
      "windows": [{
        "start": "5", "end": "20",
        "initial": {"value": {"bits": "0000"}, "changed_at": "0"},
        "changes": [{"time": "10", "value": {"bits": "0011"}}]
      }]
    }
  }
}
```

### Observation errors and semantic validation

An unsuccessful window contains **only** `start`, `end`, and `error: {kind}`; an
unsuccessful sample contains only `time` and `error: {kind}`. Observation error
kinds are `malformed` and `unsupported-signal`. `unknown-format` is allowed only
for `open`.

An error window predicts failure of an owned trace request, not when a callback
scan will fail late. Partial callbacks require separate checks and must not be
represented as successful incomplete histories. Do not hide eager/lazy detection
differences behind an arbitrary set of accepted errors: the oracle chooses a
documented observation phase. Until that phase is established for a negative
fixture, do not publish the corresponding assertion.

Path parsing errors, invalid slices, invalid handles, and runtime configuration
are checked by ordinary unit/conformance cases. Schema 1 supplies no command
language for them.

Before assertions, validation must check [oracle.schema.json](oracle.schema.json)
and the semantic constraints not captured by its structure:

- u64 limits and lossless declaration ranges;
- path and signal-ID uniqueness, resolved references, and variable lookup for
  every listed signal;
- encoding/value compatibility and bit widths;
- ranges, time-span bounds, timestamp ordering, and initial/sample timestamp bounds;
- event semantics, permitted error kinds, and consistency of overlapping
  observations.

Unobtained data must not become `null` or `[]`: omit it, or fail oracle generation.
Structural schema validation alone is insufficient.

## Validation, discovery, and execution

### Validate once before conformance

Before fixture assertions, a runner validates the selected catalog once. For
every locked provider, it must verify:

1. `$ONDAS_FIXTURES/<provider>` exists and is a directory.
2. `catalog.json` exists and obeys the catalog metadata contract.
3. Its provider identity matches both its directory and lock entry.
4. Its version exactly matches the locked version.
5. Each discovered fixture has a valid schema-1 sidecar.
6. Each sidecar references exactly one existing artifact, which is a regular file.
7. Artifact byte size and SHA-256 match the sidecar.
8. Paths stay inside the fixture directory.
9. Fixture names are unique within the provider.
10. Provenance is present and has the fields required for its kind.
11. Tags are unique whitelist members and the oracle satisfies structural and
    semantic validation.

Any validation error aborts the fixture suite before conformance tests. Validation
is separate from individual cases; do not rehash a large artifact for each sample
or trace assertion.

### Discovery and selection

Discover fixtures from immediate child directories of locked providers containing
`fixture.json`. Selection may use provider, fixture name, declared format, tags,
and backend compatibility. Selection must be explicit and deterministic; the exact
command-line interface is not part of this contract.

A selected fixture that is absent or invalid is an error, not a skip. An explicitly
requested backend-specific suite must fail if its library or required fixtures
are unavailable. An explicit fixture request must not silently succeed with zero
tests; an empty selection is an error when the suite requires at least one fixture.

### Conformance matrix

Open fixtures only through the public Ondas API, applying the same oracle to every
selected compatible backend. Sidecars describe waveforms, not backend
implementations. Where needed to guarantee adapter coverage, select the backend
explicitly; test automatic selection separately so priority changes cannot silently
remove an implementation from coverage.

The matrix is `fixture × compatible backend × supported input mode`. Where the
backend and format support bytes input, the same artifact can also be checked
through bytes-based opening. Reusable checks cover format and stable metadata,
hierarchy traversal and lookup, path parsing and canonical formatting, variables,
signals, aliases, declaration ranges, encodings, point and batch samples,
selections, traces, scans, entering values, events, bit projections, candidate
timestamps, and expected errors. Permitted false-positive candidate timestamps
are accepted, but all mandatory change times must be present.

### Configuration errors versus negative fixtures

Each of these is a fixture-run error:

- missing `ONDAS_FIXTURES` or a nonexistent root;
- missing locked provider, invalid catalog metadata, mismatched provider identity,
  or a version different from the lock;
- missing selected fixture, invalid sidecar, missing artifact, or size/checksum
  mismatch;
- unavailable required backend;
- no fixtures after selection when the suite requires at least one.

These errors must not be hidden as skips or an empty successful run. A deliberately
malformed waveform is different: it can be a valid catalog artifact with a valid
sidecar describing the expected Ondas error, which conformance then checks.

## Public/private providers and the materialization boundary

Public and private providers use the same local layout and sidecar contracts.
Every locked public artifact must be available to ordinary contributors and fork
CI without private credentials. A private fixture belongs entirely in a private
provider: public catalogs must not contain placeholders or sidecars for
inaccessible private artifacts. `public-provider/basic-values` and
`private-provider/basic-values` are distinct identities.

Private storage, authentication, and materialization are outside the runner. Once
materialized under `ONDAS_FIXTURES`, a private provider behaves like any other.
Provider source repositories and publication mechanisms are independent of Ondas.

The runner consumes local data only. It must not run simulators, execute sidecar
commands, download providers, authenticate to external storage, rebuild artifacts,
update checksums, or publish catalogs. Any external process may materialize a
provider whose resulting directory satisfies this contract, whether it contains
a tiny text waveform, a public binary corpus, a locally generated dump, a private
proprietary waveform, or a large artifact mounted from another filesystem.

Self-contained tests, shared catalog validation and conformance logic, the provider
lock contract, and an optional environment example belong on the Ondas side of
this boundary. Materialized waveform fixtures do not belong in the repository.
Delivery, storage, fixture generation, and performance methodology are outside
this contract.
