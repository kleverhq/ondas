# Fixture catalog and oracle contract

This contract defines fixture layout, provider versions, sidecars and expected
observations. [oracle.schema.json](oracle.schema.json) defines the oracle's
structure (JSON Schema Draft 2020-12); this document adds semantic constraints.
The schema does not cover catalogs or sidecar envelopes. See [testing](testing.md)
for how the runner uses them.

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

The root can hold multiple providers, each with multiple fixtures. Identity is
exactly `<provider>/<fixture-name>`; both names are single path components, and
fixture names are unique within a provider. Provider names are stable, opaque
namespaces. A repository-derived name such as `kleverhq.ondas-fixtures` is
recommended. Source URLs are not identity and are not required by the runner.

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

Fields such as `description` are allowed outside the oracle, but have no test
semantics and cannot supply materialization commands required by the runner.
The oracle's closed schema does not make the envelope a closed schema.

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

Unknown or duplicate tags fail validation. Add tags only by changing this list;
format and reader names are not tags. The contract has no tag expressions,
inheritance or profiles.

Empty oracles and tags are valid. A catalog of empty oracles can pass validation
but provides no semantic conformance evidence; the schema does not require
nonempty oracles.

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

The runner executes no oracle commands. Keep recipes, reader versions, images,
logs, source paths and oracle-generation provenance outside runtime sidecars.
Artifact provenance belongs in the envelope's `provenance`; format comes only
from `artifact.format`. Performance expectations are outside this contract.

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

## Validation and execution

### Catalog checks

Before conformance, validate each selected locked provider once:

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

Any validation error aborts before conformance. Do not rehash artifacts for
individual samples or traces.

### Discovery and selection

Discover immediate child directories containing `fixture.json`. Select by
provider, fixture name, declared format, tags or backend compatibility. Selection
must be explicit and deterministic; its CLI is outside this contract.

Missing or invalid selected fixtures, unavailable required readers, and empty
selections in suites requiring fixtures must fail rather than skip.

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

### Configuration errors and negative fixtures

Missing `ONDAS_FIXTURES` or root, failed catalog checks, unavailable required
readers and empty required selections are run errors, never skips or successful
empty runs. A deliberately malformed waveform is different: its valid sidecar
can specify the Ondas error that conformance should observe.

## Public and private providers

The private provider is `kleverhq.ondas-fixtures-private`. It is not used by public CI.

Both use the same layout and sidecars. Locked public artifacts must be accessible
to contributors and fork CI without private credentials. Keep private fixtures
entirely in private providers, without public placeholders or sidecars for
inaccessible data. `public-provider/basic-values` and
`private-provider/basic-values` are different identities.

Once installed under `ONDAS_FIXTURES`, a private provider behaves like any other.
Storage, authentication, generation and publication are external to the runner
and independent of Ondas. Any external process or mounted filesystem can supply
a provider that meets this contract, regardless of format or size.

The runner reads local data only. It cannot run simulators or sidecar commands,
download or authenticate to storage, rebuild artifacts, update hashes or publish
catalogs. Ondas owns self-contained tests, shared validation/conformance, the lock
contract and an optional environment example, not materialized waveforms.
Delivery, storage, generation and performance methodology remain outside this
contract.
