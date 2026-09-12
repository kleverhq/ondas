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

## VCD workloads

`benches/vcd.rs` uses `vcd-native` with file input and three fixed fixtures from
`kleverhq.ondas-fixtures`. All prepared queries exclude opening, signal lookup
and selection creation, and include owned result destruction.

### Swerv

`vcd0071-swerv1` measures fresh open, prepared samples
at ticks 1000 and 13000, prepared clock scans over equal-width early and late
windows, and scan versus owned trace over ticks 12000 through 13000. The latter
pair uses the same prepared one-signal selection and includes result destruction.

A four-signal comparison measures individual one-shot samples versus a single
one-shot batch at tick 13000. Both include selection creation and result
destruction, but exclude opening and signal lookup. Exact paths and inclusive
ranges are encoded in the benchmark IDs and Rust workload parameters.

Two additional prepared snapshots at tick 13000 use a fixed set of 64 unique
whole signals under `TOP.tb_top`, and `WriteData` with overlapping slices and a
repeated slice. The wide set uses explicit paths and checks handle uniqueness
before timing; it does not depend on hierarchy iteration order. Both exclude
selection creation and include owned result destruction, like the one-signal
prepared sample rather than the one-shot batch comparison.

First-change scans use the same clock and end tick 13000, starting at ticks 1
and 12000. They ignore entering state (`Initial`) and stop on the first `Change`.
Preflight requires a change to be found. Late-window prefix replay remains part
of the measurement even when the callback stops traversal early. The late case
shares its interval with the full scan. A candidate-times scan also uses that
late interval and consumes only a timestamp count; it measures the current
value-decoding path rather than assuming an activity index.

Prepared sample series execute 8 or 32 snapshots of the same clock in ascending
time, 32 ticks apart and ending at tick 13000 (starts 12776 and 12008). The existing
single prepared sample at tick 13000 is the one-query reference. Each iteration
measures the complete series, including per-query result destruction, without
retaining all snapshots. Timestamp construction and selection creation are
outside timing. Criterion reports total latency and snapshots per second. These
slow cases use a separate group with 10 samples; the other Swerv groups retain
their normal settings.

### Compact wide vector

`vcd0096-wide-compact-toggle` is 56,480 bytes but declares a 4096-bit vector with
compact payloads and activity through tick 4096. It measures fresh open and
prepared snapshots at tick 4096 of `top.wide`, its `[0:0]` slice and `top.control`.
This separates width-dependent value processing from parsing the same file.

Scans over ticks 2048 through 4096 compare the active whole vector with its
unchanging `[4095:1]` slice. A candidate-times scan uses the same whole vector and
range. All scans consume only counts. Sparse oracle windows verify representative
observations; they do not certify the entire timed interval. No full expanded
history is retained by these workloads.

### Large SCR1 recording

`vcd0097-scr1-max-ahb-coremark` is 960,716,337 bytes. It measures fresh open,
prepared `TOP.clk` snapshots at ticks 62444 and 6244000, and equal-width clock
scans over `62400..62444` and `6244000..6244044`. A prepared four-signal snapshot
at tick 6244000 selects the clock, timer, ALU result and instruction address;
exact paths are fixed in Rust. Unlike the Swerv one-shot batch comparison, this
batch excludes selection creation. The provider includes independent oracle
coverage for the late window on all four selected signals.

The SCR1 group uses 10 samples because late operations parse almost a gigabyte
per iteration. A complete run takes minutes; filter by fixture ID to restrict
timing. Fixture validation and query preflight still run for filtered-out cases.
Use the fixture provider rather than an unvalidated arbitrary input path, and
keep owned trace ranges bounded unless full-history materialization is the question.

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
