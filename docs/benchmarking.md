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
```

Select revisions with host Git and preserve Criterion output between runs;
worktrees do not share it automatically. Keep baselines in ignored build output,
without committing or archiving them. Changed fixtures or workloads invalidate a
like-for-like comparison. Repeat an unchanged revision to estimate environmental
drift; a Criterion regression label alone does not establish a code regression.

## Automation

The existing `just check` compiles benchmark targets with `cargo check --all-targets`.
For a local fixture-backed smoke run, use
`./dev cargo bench --locked --bench vcd -- --test`. Smoke timings are not
regression evidence. A proprietary
target must not prevent unrelated public targets from compiling; explicitly
requesting that target without its required fixtures or runtime must fail clearly.
Optional private payload absence follows [fixture policy](fixtures.md): omit those
cases before registration and report the omission, rather than timing an empty
operation. Installed but invalid inputs always fail.

Keep Criterion's runner, results and comparisons. Custom history stores,
thresholds, dashboards and mandatory cross-format datasets are outside this
contract, as are memory, allocation and RSS profiling. Put notes in ignored `tmp/`;
use tracked WIP only when the investigation must travel with a branch.
