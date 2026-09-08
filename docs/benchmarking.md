# Benchmarking

Ondas uses Criterion for manual investigation of public API performance.
Criterion owns local results and baselines. Performance is not an automatic CI
or release gate, and benchmarks do not duplicate functional conformance checks.

## Workload Organization

Use one Criterion target per measured format, such as `benches/fst.rs`. Share
fixture lookup and operation implementations in `benches/support/`; keep concrete
fixtures, reader choices, and workload parameters in the format target.

```text
benchmark target = format
benchmark case   = fixture + operation + parameters
backend          = reader implementation
```

Equivalent fixtures across formats are not required, and measurements from
different format targets are not automatically comparable. Add a target or case
to answer a concrete performance question, not to complete a parameter matrix.

Use the same fixture identity, provider-version lock, and materialized catalog
as [tests](fixtures.md). Reuse catalog validation, including artifact integrity,
before measurement, not inside timed iterations. Benchmark code neither fetches
fixtures nor updates their metadata.

Signal paths, selection sizes, ticks, ranges, and projections are Rust workload
parameters. Do not introduce a benchmark manifest, recipe language, or
performance expectations in fixture sidecars.

## Controlled Comparisons

Use an explicit backend for the principal cases. Compare readers within one
Criterion group over the same artifact, operation, selection, time bounds, and
setup boundary. Automatic opening is a separate workload, not a substitute for
explicit reader comparison.

Criterion IDs must be globally unique across targets and describe format,
provider/fixture identity, operation, input mode, and relevant parameters. Reader
names distinguish cases inside a shared group. File and bytes inputs are modes
of the same format target; register bytes cases only for readers that support
them.

Before comparing revisions, run relevant unit and conformance tests. Benchmark
setup checks fixture availability, backend availability, signal resolution, and
successful operations. It does not reimplement oracle assertions, compute result
fingerprints, or infer correctness from matching timings.

## Measurement Boundaries

Each case states what its timed region includes:

| Operation | Timed work | Preparation outside measurement |
|---|---|---|
| Open | Create and drop a fresh waveform each iteration | Resolve and validate the fixture |
| Hierarchy traversal or lookup | The selected traversal or lookup | Open the waveform; prepare query inputs |
| Select one or many | Construct and drop the selection | Open the waveform and resolve signal handles |
| Point or batch sample | The query and consumption of its result | Open, resolve handles, prepare time and selection |
| Scan a narrow or full range | Scan with a minimal callback consumer | Open, resolve, prepare range and selection |
| Trace a whole or projected signal | Materialize and consume the trace | Open, resolve projection, prepare range |
| Candidate times | Collect and consume candidates | Open and prepare the selection and range |

Prepared-query cases reuse the waveform and selection. Do not accidentally
include path resolution or selection creation unless that is the operation under
study. Use `black_box` on inputs and results; for callback scans, consume a minimal
counter or equivalent observable output through `black_box`.

Opening a fresh waveform is not a cold-disk benchmark: filesystem page cache may
be warm. Reusing query preparation likewise measures that reuse, not an
unprepared first query. Keep setup and destruction boundaries consistent between
compared cases.

## Local Baselines

Use Criterion's normal baseline mechanism for an existing format target, for
example from the repository root:

```sh
# On the baseline revision:
./dev cargo bench --bench fst -- --save-baseline old
# On the comparison revision, retaining the same local Criterion results:
./dev cargo bench --bench fst -- --baseline old
# Restrict the workload when investigating a specific operation:
./dev cargo bench --bench fst -- scan
```

Revision selection is a host Git operation. Preserve local Criterion results
between runs; separate worktrees do not inherently share baselines. Baselines
remain under ignored build output and are neither committed nor archived by the
project. Changed fixtures or workload definitions invalidate an assumed
like-for-like comparison.

## Automation Boundary

CI may compile benchmark targets with `cargo check --benches` and may perform a
public-fixture smoke run. Smoke timings are not regression evidence. A proprietary
target must not prevent unrelated public targets from compiling; explicitly
requesting that target without its fixtures or runtime must fail clearly.

Do not add a custom runner, result format, comparator, historical result store,
threshold gate, dashboard, or mandatory cross-format dataset. Memory, allocation,
and RSS profiling are separate investigations, not part of this benchmark
contract. Use ignored `tmp/` for ad hoc notes and the tracked-WIP policy only when
investigation context needs to travel with a branch.
