# Benchmarking

Use Criterion to measure public API workloads and compare local baselines.
Timings are not CI or release gates. Run correctness checks outside timing.

## Workloads and comparisons

Use one target per format in `benches/<format>.rs`. Keep fixture choices,
readers, signal paths, bounds and sample settings in Rust source. Share artifact
checks with `tests/support/fixtures.rs`. Extract consumers only when another
target needs them. Do not add benchmark manifests or performance expectations
to fixture sidecars.

Use the [pinned corpus](fixtures.md). Validate hashes, resolve handles and check
workload assumptions before timing, including filtered runs. Benchmarks do not
fetch fixtures, update metadata or repeat conformance assertions. Sparse oracle
evidence covers only its declared observations.

Use an explicit reader for principal cases. Compare the same artifact,
operation, selection, bounds and setup boundary. Automatic opening is a separate
case. IDs must be unique across targets. Start group IDs with
`<format>/<fixture>` and include the operation, input mode and relevant
parameters; reader names distinguish competitors. Keep supported file/bytes
modes in the same format target.

Choose comparisons that answer a concrete question:

| Question | Comparison |
|---|---|
| Prefix work | Equal-width early and late windows; short and long histories with equal early activity |
| Setup and reuse | Fresh readers, fresh selections, prepared repeats and forward/backward query series |
| Batch work | Individual and batched points; count unique bases separately from aliases, projections and repeated positions |
| Type-dependent paths | Imported and authored declarations; qualified, unqualified and mixed point batches |
| Unrelated data | Equal selected histories with more declarations or denser unselected activity |
| Width and projection | Whole vectors, changing low bits, stable high slices and equivalent scalars |
| Output and stopping | Scan counters, candidate times, owned traces and early stops on the same window |
| Composed queries | Selective/eager payload access, shared/distinct drivers and union/disjoint calls retaining earlier results |

Combine real recordings with controlled inputs. File size or a large end tick
does not establish selected activity. Keep each comparison focused; formats do
not need a complete matrix or matching datasets. Bound owned traces unless full
materialization is the question. Adjust sample counts for slow work, not the
operation itself.

Cross-format comparisons require equivalent selected histories and consumers.
The FSDB/SHM `matched-clock` cases alternate one bit through tick 1048576 with
matching query boundaries. Their time units and surrounding designs differ;
compare records and tick positions, not physical durations or total file sizes.

## Measurement boundaries

State the timed work and keep other setup outside it:

| Operation | Timed work | Untimed preparation |
|---|---|---|
| Open | Create and drop a fresh waveform | Validate the artifact |
| Hierarchy or lookup | Traverse or look up | Open; prepare names and lookup state |
| Select | Construct and drop a selection | Open; resolve handles |
| Samples | Query and consume results | Open; resolve handles and time |
| Scan | Traverse with a minimal counter | Open; resolve handles and range |
| Traces | Create, consume and drop output | Open; resolve handles, projections and range |
| Candidate times | Collect and consume candidates | Open; prepare selection and range |

A `fresh-selection` case includes selection creation, query and cleanup in each
iteration. A prepared case creates its selection outside timing; a
`warm-repeat` case also primes it. Repeats can measure a retained-state hit
rather than native traversal. Measure first entry separately. First/second
lookup cases open a fresh waveform outside timing; the second performs one
lookup before its timed lookup.

For supported byte inputs, load shared owned bytes before timing. Opening may
clone that handle, but must not include file loading unless stated.
Use `black_box` on inputs and results. Sample-series throughput counts queries,
not returned records; FSDB/WLF `block8` cases repeat one eight-timestamp block.

Include result destruction and query cleanup, including after an early stop.
A callback stop does not measure decoding only the delivered records: parsing,
SDK loading or native collection may precede callbacks. Selective payload access
does not prove skipped decoding. A fresh waveform can use a warm filesystem
cache; it is not a cold-disk test. Timing alone does not identify SDK memory use,
allocations, RSS or lock contention.

## Format-specific interpretation

Concrete cases are in the sources below. Backend documents own traversal,
retained-state and SDK constraints.

| Format | Sources | Main controls |
|---|---|---|
| VCD | [vcd.rs](../benches/vcd.rs) | Real recordings, prefix replay, batching, width and output volume |
| FST | [fst.rs](../benches/fst.rs), [hotpaths](../benches/fst/hotpaths.rs), [composed](../benches/fst/composed.rs) | Selected activity, section boundaries, unrelated topology and conditional payloads |
| GHW | [ghw.rs](../benches/ghw.rs) | Opening/index work, topology, projections, repeated and composed queries |
| FSDB | [fsdb.rs](../benches/fsdb.rs), [controlled](../benches/fsdb/controlled.rs), [typed](../benches/fsdb/typed.rs), [composed](../benches/fsdb/composed.rs) | History length, topology/aliases, width, typed records and conditional reads |
| SHM | [shm.rs](../benches/shm.rs), [cold](../benches/shm/cold.rs) | Type qualification, cold/mixed batches, late windows, topology and disjoint calls |
| WLF | [wlf.rs](../benches/wlf.rs), [controlled](../benches/wlf/controlled.rs) | History length, topology/aliases, projections and repeated queries |

[VCD](vcd-native.md) and [FST](fst-lib.md) can traverse old selected activity
for a narrow late query. FST section access can also require chain or frame
decompression before a callback. Compare full scans with narrow windows and
early stops without treating them as direct lookup costs.

[GHW](ghw.md) opening includes body validation and optional payload-index work.
Report opening alongside the first query. The wellen reference distinguishes
header/body and selected-history loading from retained-history queries.
Unsupported types belong in conformance, not replacement-value benchmarks.

[FSDB](fsdb-lib.md) prepared queries can still load SDK data; a repeated point
can avoid traversal through retained selected state. Projection change times
can require backward work. The typed shortreal source is stored as binary64;
the separate native-real32 fixture checks four-byte conversion.

[SHM](shm-lib.md) certification can process source history inside SST2.
Unqualified histories collect ordinary prefixes; `Break` stops delivery after
native collection. The cold cases create fresh selections for one/four
qualified bases, a mixed five-base batch, imported wires and 100 distinct IDs.
They include equal-width early/late windows and union versus two disjoint reads
that retain the first trace. Keep prepared repeats separate.

[WLF](wlf.md) timings include worker communication when SDK work is needed.
Cold points and windows establish entering state; unresolved projected change
times can require chronological fallback. Prepared repeats can avoid that work.
These cases do not compare Writer and simulator storage policies.

## Local baselines

Use the pinned development toolchain; the library MSRV check is separate.
Install fixtures explicitly, run conformance, then execute benchmark smoke before
timing on an idle machine. Public targets use:

```sh
./dev just fixtures-install
./dev just conformance
./dev just bench-smoke
# Baseline revision:
./dev cargo bench --locked --bench vcd -- --save-baseline old
# Comparison revision, retaining the same Criterion output:
./dev cargo bench --locked --bench vcd -- --baseline old
# Focus on one operation:
./dev cargo bench --locked --bench vcd -- scan
```

For vendor targets, configure the [local SDK selection](automation.md#proprietary-environments):

| Format | Correctness | Execution smoke | Timed run, accepts Criterion arguments |
|---|---|---|---|
| FSDB | `./dev just conformance-fsdb` | `./dev just bench-smoke-fsdb` | `./dev just bench-fsdb --save-baseline old` |
| SHM | `./dev just conformance-shm` | `./dev just bench-smoke-shm` | `./dev just bench-shm --save-baseline old` |
| WLF | `./dev just conformance-wlf` | `./dev just bench-smoke-wlf` | `./dev just bench-wlf --save-baseline old` |

Use `--baseline old` on the comparison revision. Each selected SDK has a separate
target directory; keep SDK list order stable or use fresh output.
`./dev just bench-build-wlf` compiles without running Criterion.

Select revisions with host Git. Worktrees do not share Criterion output
automatically. Keep results in ignored build output, not Git. Record code and
fixture commits, SDK identity and the loaded runtime in ignored local evidence.
Repeat an unchanged revision to estimate drift. A Criterion regression label
alone does not establish a code regression.

### Fixture revisions

Tests and benchmarks use the same `fixtures/` submodule checkout. Record its
full SHA. Changed artifacts, workloads or group IDs require new baselines;
comparisons need matching IDs and corpus revisions. Local submodule edits are
development evidence. Shared results need an available fixture commit and its
parent Git entry. See [corpus updates](fixtures.md#updating-the-corpus).

## Automation

`just check` compiles public benchmark targets. Default checks exclude the
feature-gated FSDB, SHM and WLF targets. Explicit SDK checks must compile and run
the requested target; missing fixtures or runtimes must fail clearly.

`./dev just ci` runs public conformance and VCD/FST/GHW smoke.
`ci-fsdb`, `ci-shm` and `ci-wlf` run their smoke after vendor conformance.
Criterion `--test` checks execution only; compilation and smoke supply no
timing comparison or speed threshold.

FSDB preflight validates artifacts, then prints `SKIP` and omits groups when
the reader rejects a newer file version. Other errors fail. Skipped groups
supply no evidence. SHM has no newer-file skip rule.
See [fixture execution rules](fixtures.md#validation-and-execution).

Keep Criterion's runner and comparisons. Custom history stores, dashboards,
timing gates and mandatory cross-format datasets are outside this policy.
