# Benchmarking

Use Criterion to investigate public API performance and manage local results and
baselines. Timings are not CI or release gates; benchmarks do not repeat
conformance assertions.

## Workloads

Use one Criterion target per measured format, such as `benches/vcd.rs`. Share
catalog and artifact validation with conformance tests in
`tests/support/fixtures.rs`. Keep concrete fixtures, reader choices, operations,
and workload parameters in the format target; extract shared operations only
when another target needs them.

```text
benchmark target = format
benchmark case   = fixture + operation + parameters
backend          = reader implementation
```

Add cases for concrete performance questions, not to fill a matrix. Formats need
neither equivalent fixtures nor directly comparable results.

Use the [test catalog and lock](fixtures.md). Validate fixtures and their hashes
before timing, never inside iterations. Benchmarks neither fetch fixtures nor
update metadata.

Signal paths, selection sizes, ticks, ranges, and projections are Rust workload
parameters. Do not introduce a benchmark manifest, recipe language, or
performance expectations in fixture sidecars.

## Comparisons

Use an explicit backend for the principal cases. Compare readers within one
Criterion group over the same artifact, operation, selection, time bounds, and
setup boundary. Automatic opening is a separate workload, not a substitute for
explicit reader comparison.

Criterion IDs must be globally unique across targets and describe format,
provider/fixture identity, operation, input mode, and relevant parameters. Reader
names distinguish cases inside a shared group. File and bytes inputs are modes
of the same format target; register bytes cases only for readers that support
them.

Run relevant unit and conformance tests before comparing revisions. Setup checks
fixtures, readers, signal resolution and successful operations. It does not repeat
oracle assertions, fingerprint results or infer correctness from equal timings.

## Measurement boundaries

Each case states what its timed region includes:

| Operation | Timed work | Preparation outside measurement |
|---|---|---|
| Open | Create and drop a fresh waveform each iteration | Resolve and validate the fixture |
| Hierarchy traversal or lookup | The selected traversal or lookup | Open the waveform; prepare query inputs |
| Select one or many | Construct and drop the selection | Open the waveform and resolve signal handles |
| Point or batch sample | The query and consumption of its result | Open, resolve handles, prepare time and selection |
| Scan a narrow or full range | Scan with a minimal callback consumer | Open, resolve, prepare range and selection |
| Trace a whole or projected signal | Materialize and consume the trace | Open, resolve projection, prepare range and selection |
| Candidate times | Collect and consume candidates | Open and prepare the selection and range |

Prepared-query cases reuse the waveform and selection. Do not accidentally
include path resolution or selection creation unless that is the operation under
study. Use `black_box` on inputs and results; for callback scans, consume a minimal
counter or equivalent observable output through `black_box`.

A fresh waveform may use a warm filesystem cache; it is not a cold-disk test.
Prepared queries measure reuse, not first-query cost. Compare cases with the same
setup and destruction boundaries.

## VCD workload design

Combine real recordings with compact diagnostic fixtures. Real recordings show
whether a cost matters in practical analysis; controlled activity isolates a
cause that total file size or a mixed workload can hide. The
[VCD backend model](vcd-native.md) explains the parsing and replay constraints.
Choose comparisons along these independent dimensions:

- **History traversed versus window width.** Equal-width windows at different
  positions distinguish output volume from the work needed to establish entering
  state. Early termination should be tested with both short and long prefixes:
  finding few changes does not necessarily require reading little history.
- **Shared work versus repeated queries.** Sampling several signals at one time
  tests batching; sampling at several times tests reuse across queries. Neither
  substitutes for the other. Distinguish unique base signals from projections
  and repeated selection entries when varying selection size.
- **Encoded size versus logical width.** Compact text can represent wide values.
  Whole-vector, slice and narrow-control queries over the same activity reveal
  width-dependent processing without changing the amount of source data. An
  unchanged slice of an active vector distinguishes processing cost from the
  number of emitted changes.
- **Traversal versus result production.** Callback scans, owned histories and
  activity timestamps ask for different amounts of output. Comparing equivalent
  selections and intervals reveals whether requesting less output avoids work
  or merely discards its result.

Keep each comparison focused rather than multiplying every dimension into a
matrix. Bound owned-history ranges unless full materialization is the question,
and choose sample counts appropriate to slow workloads rather than shortening
away the expensive operation. Sparse oracle coverage establishes correctness only
for its declared observations, not for every interval used in timing. Concrete
fixtures, paths, bounds and measurement settings belong in `benches/vcd.rs`.

## FST workload design

`benches/fst.rs` measures the public API with the explicit `fst-lib` reader,
not the decoder dependency in isolation. Real recordings cover a compact design
with every unique history selected, a medium recording with focused queries, and
a larger recording with a small selection. They distinguish total artifact size
from selected activity without requiring a cross-format comparison.

The [FST backend model](fst-lib.md) explains why section seeking does not
imply direct lookup at the requested start tick. Equal-width early and late
windows expose replay from zero; individual and batched samples expose shared
traversal; sample series measure repeated queries without a history cache.
Early-stop cases measure the public callback boundary, including any chain or
frame decompression that precedes that callback. Do not interpret them as the
cost of decoding only one change.

Compare scans, candidate times and owned traces on the same narrow window.
Whole-vector and slice traces isolate projection and result-production costs;
repeated, overlapping projections distinguish selection entries from unique base
reads. Full scans use a callback counter; full owned histories are limited to the
compact recording. Hierarchy and selection cases operate on an already opened
waveform and do not include decoding values.

A few file/bytes pairs cover opening and prepared queries. Bytes are loaded into
shared owned storage before timing; opening includes cloning that shared handle
but not reading the file into memory. Both modes include waveform destruction in
open measurements and reuse their selections in prepared-query measurements.

The controlled workloads in `benches/fst/hotpaths.rs` separate selected activity
from full-design overhead. Three recordings retain identical sparse and constant
probe histories while varying either unselected handle count or global time-table
density. Quiet windows and first-change scans expose work performed even when
few or no changes are returned. Constant-value samples immediately around real
SCR1 section boundaries isolate section setup from selected transitions.

A compact 4096-bit recording pairs a toggling low bit with an equivalent scalar.
Whole-vector, low-bit and stable high-slice queries distinguish base-width work
from projection work and output volume. Repeated low-bit selection entries share
a base read while retaining separate results. Candidate-time and owned-trace
cases use the same windows as their scan counterparts. Fixture conformance checks
the independent histories; benchmark preflight checks ensure that the intended
active and quiet cases remain distinct.

Concrete fixtures, paths, bounds and sample counts remain in `benches/fst.rs`
and `benches/fst/hotpaths.rs`.

## FSDB workload design

`benches/fsdb.rs` measures the public API with the explicit `fsdb-lib` reader
and file inputs only. The target requires the `fsdb-lib` feature, the SDK runtime
and the locked public fixture provider; it does not use private fixtures. See
[FSDB setup and limits](fsdb-lib.md) for the SDK environment.

Three compact recordings serve different questions: `fsdb0004-compare` covers
open, hierarchy traversal/lookup, selection construction, four distinct base
signals sampled individually, batched or through a prepared selection, and
bounded full scans, candidate times and owned traces. `fsdb0003-mode-change`
compares equal-width early/late clock windows, early callback termination,
quiet windows on sparse and constant signals, and repeated point queries.
`fsdb0009-wide-bus` compares a 1024-bit base, its changing low bit and a stable
slice on the same window, plus repeated low-bit selection entries. Each recording
also has an open/drop case. Workload preflight distinguishes active windows from
quiet windows; independent value correctness belongs to fixture conformance.

The [FSDB backend model](fsdb-lib.md) explains why prepared selections do not
imply cached histories: every query loads selected signals into the SDK, creates
a traversal handle, starts from the beginning and unloads afterward. That SDK
work is included in prepared-query measurements. Early callback termination
cannot avoid loading that precedes the first callback. Selection construction
alone does not load values. One-shot individual and batched samples both include
selection creation and owned result destruction; prepared samples exclude only
the selection setup. Scan and candidate callbacks count records without storing
them; traces include materialization and destruction. A query-series iteration
contains 8 or 32 point queries by repeating the same eight-timestamp block once
or four times, with throughput reported in queries rather than returned records.
The `block8` series IDs distinguish these workloads from earlier series with
different timestamp distributions. Artifact validation and semantic preflight
checks still run during registration, including filtered runs; redundant
success-only query probes are omitted.

Those three recordings are only approximately 8–35 KB, and their wide base has
just two post-initial changes. Controlled public recordings supplement them in
`benches/fsdb/controlled.rs` and `benches/fsdb/typed.rs`:

| Fixtures | Controlled comparison |
|---|---|
| `fsdb0010-history-short`, `fsdb0011-history-long` | Identical early queries across history lengths; late replay; selected base counts; batch/prepared/individual samples; repeated queries; matched full traversal and early stop |
| `fsdb0012-topology-small`, `fsdb0013-topology-many-handles`, `fsdb0014-topology-many-times` | Equal sparse/constant probes with more unrelated identities or denser unrelated activity |
| `fsdb0015-wide-compact-toggle` | Sustained 4096-bit LSB/MSB changes versus a scalar, changing/stable slices, borrowed/owned samples and whole-vector/slice duplicates |
| `fsdb0016-topology-many-aliases` | More declarations at fixed unique-history count; true alias identity and per-selection-entry costs |
| `fsdb0017-typed-records` | Four-state logic, events, real values and fixed-length short/long byte strings |
| `fsdb0018-native-real32` | Verified four-byte real storage and scalar control |

The history pair separates an unused suffix from an increasingly long traversed
prefix without assuming how much the SDK decompresses internally. Output-heavy
operations use bounded windows even on the long recording. Wide LSB/MSB cases
have matching widths and change counts but different first differing positions
in the shared MSB-first value comparison. Duplicate whole vectors exercise
retained-state copying which one-bit duplicates do not represent. Borrowed
sample output still includes decoded/retained state; it is not an allocation-free
backend measurement. The typed file's `top.real32` is a shortreal source variable
stored as binary64 by its producer; the separate native-real32 recording covers
the four-byte conversion path.

These are workload comparisons, not profiler attribution. SDK memory usage,
allocation attribution and concurrent lock contention require separate
measurement methods. A large numerical end tick is not a substitute for many
recorded changes. Keep exact paths, bounds and sample settings in the benchmark
sources, and verify fixture conformance before interpreting timings.

## Local baselines

Use Criterion's normal baseline mechanism for an existing format target, for
example from the repository root:

```sh
# On the baseline revision:
./dev cargo bench --locked --bench vcd -- --save-baseline old
# On the comparison revision, retaining the same local Criterion results:
./dev cargo bench --locked --bench vcd -- --baseline old
# Restrict the workload when investigating a specific operation:
./dev cargo bench --locked --bench vcd -- scan
# Capture an FST baseline with the same Criterion mechanism:
./dev cargo bench --locked --bench fst -- --save-baseline fst-initial
# Focus on controlled topology or width-sensitive workloads:
./dev cargo bench --locked --bench fst -- topology
./dev cargo bench --locked --bench fst -- wide-compact-toggle
# Public FSDB corpus, with the SDK feature enabled:
./dev cargo bench --locked --features fsdb-lib --bench fsdb -- --save-baseline fsdb-public-initial
./dev cargo bench --locked --features fsdb-lib --bench fsdb -- --baseline fsdb-public-initial
```

Select revisions with host Git and preserve Criterion output between runs;
worktrees do not share it automatically. Keep baselines in ignored build output,
without committing or archiving them. Changed fixtures or workloads invalidate a
like-for-like comparison. Repeat an unchanged revision to estimate environmental
drift; a Criterion regression label alone does not establish a code regression.

### Unpublished local providers

A locally generated provider can be used without publishing it. Its catalog
version must still match `fixtures.lock.toml`, and every referenced sidecar and
payload must be present. Keep a self-contained snapshot under ignored `tmp/`
rather than running against a producer directory while it is being modified.
The tagged-release installer is not applicable to an unpublished version.

For a snapshot at `tmp/fsdb-fixtures/kleverhq.ondas-fixtures`, select its parent
inside the existing worktree container without changing canonical environment
configuration:

```sh
./dev bash -c 'export ONDAS_FIXTURES="$PWD/tmp/fsdb-fixtures"; cargo test --locked --features fsdb-lib --test conformance full_fsdb_pool -- --ignored --nocapture'
./dev bash -c 'export ONDAS_FIXTURES="$PWD/tmp/fsdb-fixtures"; cargo bench --locked --features fsdb-lib --bench fsdb -- --test'
```

Use the same override for timing and fixture-backed quality checks. A lock update
to an unpublished provider is local integration work, not evidence that others
can install it from a release.

## Automation

The existing `just check` compiles benchmark targets with `cargo check --all-targets`.
For a local fixture-backed smoke run, use
`./dev cargo bench --locked --bench vcd -- --test` or
`./dev cargo bench --locked --bench fst -- --test`. For FSDB, first run public
conformance, then smoke the feature-gated target:

```sh
./dev cargo test --locked --features fsdb-lib --test conformance full_fsdb_pool -- --ignored --nocapture
./dev cargo bench --locked --features fsdb-lib --bench fsdb -- --test
```

Default all-target checks skip FSDB's feature-gated target; SDK-enabled checks
must compile it explicitly. Smoke timings are not regression evidence. A proprietary
target must not prevent unrelated public targets from compiling; explicitly
requesting that target without its required fixtures or runtime must fail clearly.
Optional private payload absence follows [fixture policy](fixtures.md): omit those
cases before registration and report the omission, rather than timing an empty
operation. Installed but invalid inputs always fail.

Keep Criterion's runner, results and comparisons. Custom history stores,
thresholds, dashboards and mandatory cross-format datasets are outside this
contract, as are memory, allocation and RSS profiling. Put notes in ignored `tmp/`;
use tracked WIP only when the investigation must travel with a branch.
