# SHM query cost and coverage

## Scope and evidence

The investigated source is Ondas 1.1.0, commit
`1cf6a0e61f645ad70f507bab8c20fa03c78876b0`. The investigation branch is
`bb/shm-performance-thr_9ubsj62rqv`. The fixture pin is
`4ce2f4ea6a034f1db335cc96ba9e67e6dcbb2a38`.

A standalone release consumer calls the common Ondas API, without wavepeek.
It uses published `ondas = "=1.1.0"`, `shm-lib,fsdb-lib`, and the handoff lockfile.
The published native shim, Rust adapter, query engine and SHM benchmark source
have the same SHA-256 as this checkout. Diagnostic instrumentation exists only
in an ignored source copy. No library algorithm or public contract is changed.

Local evidence is under `tmp/shm-investigation/`. It includes consumer sources,
inputs, raw results, phase logs, corpus/type inventories, loader checks and
validation logs. SDK paths, versions, configuration and conversion provenance
remain there. Eight SHM/FSDB inputs are preserved there, outside Git and Docker
build contexts. Their source FST sidecars match the pinned corpus. The SHM
conversion stamps identify FST input and the converter; preparation uses a VCD
intermediate. The retained FSDB preparation evidence also uses FST-to-VCD
conversion. Backend identity, source span and tick units were checked at runtime.
The representative pairs use 1-ps ticks and matching spans.

`tmp/wavepeek-perf-handoff-20261016/` preserves the supplied evidence. All 339
manifest hashes pass. Its 144 direct Hyperfine pairs each contain five successful
runs per format; recomputing their medians confirms 51 ratios above 2 and the
maximum 44.218. These are historical CLI measurements, not fresh measurements
of this checkout. This investigation reruns representative native operations,
not all 144 CLI pairs. It does not import the wavepeek source changes.

## Reproduced costs

The following are medians of three serial release runs on an idle machine,
after builds and conversion. Opening and simplified resolution precede the
query timer; waveform destruction follows it. Filesystem caches can be warm.
These are query costs, not cold-disk or CLI measurements.

| Common API operation | SHM query, s | FSDB query, s |
|---|---:|---:|
| PicoRV32 samples, 1 position | 0.1008 | 0.0038 |
| PicoRV32 samples, 100 positions | 1.8897 | 0.0342 |
| PicoRV32 samples, 1000 positions | 6.7135 | 0.1104 |
| DualRocket samples, 100 positions | 3.4718 | 0.0619 |
| PicoRV32 traces, 95 paths, late 8-us window | 1.8673 | 0.0447 |

PicoRV32 opening is about 0.8 ms. Its 1000-position input contains **419 distinct
names**, repeated to fill 1000 positions. Native collection receives 419 IDs.
The selected SDK lacks alias identity, so this is not evidence for interning
1000 different declarations into 419 confirmed physical aliases. Destruction
adds about 79 ms in that case. DualRocket opening is about 37 ms and the
100-position sample adds about 53 ms of destruction.

The late PicoRV32 window returns 16,318 changes. A scan costs 1.9002 s; a composed
query stopped after 50 callbacks costs 1.9018 s. An early window with the same
selection and tick width costs 0.0064 s. The early and late outputs differ in
activity; phase counts below establish the prefix cost independently of that
difference. Exact inclusive input bounds are retained in the local JSON inputs.

| 32-us selection, native open + query + drop | Union, s | Trigger then remaining paths, s |
|---|---:|---:|
| ClusteredRocket, 100 paths | 1.0191 | 1.1372 |
| DualRocket, 100 paths | 0.6463 | 0.6942 |
| DualRocket, 10 paths plus trigger | 0.5201 | 0.5708 |

All entering values, entering `changed_at` values and changes match the union
read and FSDB: 96,602 / 64,801 / 64,101 changes. The retained trigger trace is not
read again as payload. This measures a valid current call sequence, not a lower
bound over possible algorithms. The last case remains borderline: the supplied
20-run CLI confirmation is 1.984x, with a 0.592-s budget. The other two staged
failures and 48 single-query over-budget cases are separate categories in the
handoff; this investigation does not infer them from an aggregate median.

The six SCR1 single-clock windows cost 44.1–45.8 ms for native open + traces +
drop. Raw traces match FSDB in all six. Their supplied direct CLI budgets are
29–31 ms. Direct native timings confirm the small costs; the earlier Python
wrapper's wait polling explains why those ratios were masked. Wrapped/direct
ratios are not code speedups.

Raw bits match in all four rerun point selections. Full trace comparisons also
cover the PicoRV32 window, six SCR1 windows and three staged selections. They
compare representation and timestamps, including entering `changed_at`, not
only counts or an expansion of lossy mixed-X hexadecimal text.

## Confirmed causes

### Type rejection routes whole batches through ordinary collection

[`point_type`](../../../native/shm.cpp) at line 259 checks exact SST2 fields.
These observed scalar representations differ on the **same reader**:

| Field selector | Accepted authored scalar | PicoRV32 converted scalar |
|---|---:|---:|
| kind, `da_get(0x173)` | 2 | 2 |
| type code, `da_get(0x1c3)` | 3 | 5 |
| properties, `da_get_int(0x1c4)` | `0x4060a` | `0x7004060a` |
| format, `da_get(0x1b3)` | 1 | 1 |
| storage bits, `da_get_int(0x164)` | 2 | 8 |

The scalar branch rejects the different code and properties. In the 419-ID
PicoRV32 selection, 252 scalars and four 32-bit declarations fail qualification;
163 packed histories pass `point_type`. The four rejected 32-bit declarations
have properties `0x10061c` and format 3, versus required `0x10060c` and format 4.
One rejected history prevents certification of the entire batch
(`ondas_shm_sample_bits`, line 488). Thus eligible packed histories also use
ordinary collection. Eligibility alone does not prove that their subsequent
availability certificate would succeed.

All four slow point selections reject before `point_prefix` (line 283).
Measured qualification is below 2 microseconds; prefix calls are zero. Expensive
equality-expression certification is **rejected as the cause of these cases**.
A qualified authored million-tick scalar does call the certificate and costs
about 25 ms; that separate cost must not be generalized to the rejected points.
An independently built second SDK shows the same scalar field difference and
accept/reject outcome. This is not an accidental loader selection or a behavior
seen only with one SDK. It is not a complete SDK compatibility/performance matrix.

### Prefix traversal and merge dominate cold fallback

`history` (line 352) loads a cursor, starts at the beginning without validated
resume, decodes records through the inclusive end, and retains the selected
prefix. `ondas_shm_read` (line 451) merges complete histories before Rust
delivery. The preparing point fallback reuses its already-open session and
hands collected records to the matching read; it does not collect them twice.

One instrumented run gives the following phase costs. These are diagnostics,
not additional statistical medians. History includes SDK cursor work, decoding
and copying; it does not isolate SDK internals.

| Case | Collected records | History, s | Merge, s | Whole query, s |
|---|---:|---:|---:|---:|
| PicoRV32 point, 1 | 491,067 | 0.065 | negligible | 0.095 |
| PicoRV32 point, 100 | 5,560,215 | 1.047 | 0.487 | 1.902 |
| PicoRV32 point, 419 IDs | 16,652,597 | 3.311 | 1.905 | 6.703 |
| DualRocket point, 100 | 10,653,766 | 1.986 | 0.824 | 3.443 |
| PicoRV32 late window | 5,522,725 | 1.045 | 0.491 | 1.904 |

The remaining time includes native ordering checks, Rust delivery/normalization
and cleanup; it has not been split further. A repeated prepared PicoRV32 window
collects only 16,318 records and takes 0.0118 s. Fresh one-shot calls do not retain
that selection checkpoint. Native collection still contains 5,522,725 records
when the public query breaks after 50 callbacks.

[`src/backends/shm.rs`](../../../src/backends/shm.rs), line 408, calls native
collection before its visitor loop. `Break` then stops delivery and releases
records. It cannot undo collection. The generated-reader advance-budget tests
do not observe this native buffer.

### Repeated session and hierarchy setup also matters

Every non-reused native query operation constructs `Session` (line 49) and imports
the full hierarchy. No vendor handles survive the operation. This supports
callback reentrancy, different time units and thread transfer.

Cold positive-start windows first try to establish entering state
([`src/query/engine.rs`](../../../src/query/engine.rs), line 990,
`point_states(..., false)`). A rejected type closes that session without preparing
ordinary records. The subsequent ordinary read opens and imports again.
A staged trigger/payload workflow can therefore perform four imports after the
initial open, even when selections do not overlap.

In one ClusteredRocket staged run, those four imports cost about 0.186 s and
session open/close about 0.034 s. Trigger history collection costs only 0.020 s.
Payload prefix collection and merging remain substantial. DualRocket shows the
same sequence with smaller import costs. These are several costs, not a claim
that hierarchy alone explains the multi-second point cases.

## Why the existing coverage permits this

The pinned SHM pool has 78 fixtures: 66 imported and 12 authored. All have
nonempty value oracles. The native inventory finds the rejected scalar
properties in 40 imported databases and in oracle-listed selections in 39.
The data class is therefore **not absent from correctness coverage**.

However, all 66 imported oracles have zero `samples` assertions. Their 1,010
windows have only six positive starts, all single-tick UART windows. The dense
late-prefix and cold-point combination is not represented by these oracles.
`pool_queries` consumes the listed evidence; it does not automatically derive
extra cold point scenarios from every complete window.

| Actual check or workload | What it checks; why it does not reject this cost |
|---|---|
| Public CI and release, `.github/workflows/ci.yml`; `justfile:194,203`; Cargo feature gates | Public `ci` runs nonvendor tests/conformance and VCD/FST/GHW smoke. It does not enable SHM. A release tag repeats this public gate. |
| Explicit `ci-shm`, `justfile:110` | Runs real SHM conformance, feature-enabled checks, MSRV and standalone consumers, then Criterion `--test`. It has no time or work threshold. The preserved release gate actually ran one selected SDK successfully; it was not silently omitted. |
| SDK sweep, `tools/repo/shm-sweep:5` | Runs every locally selected SDK, fails missing readers and any selected failure. It requires no fixed SDK list. Export/initializer/alias tests check ABI discovery, not type-performance eligibility. A second SDK control here reproduces the rejection. |
| `full_shm_pool`, `tests/conformance.rs:2582`; `pool_queries:1358` | Compares sparse oracle observations through common APIs. Correct chronological fallback passes. Imported histories are mostly checked from tick zero, without cold point assertions. No native collection counter or timer is asserted. |
| Authored/workload consumers, `tests/support/shm_workload_tests.rs:16` | Check formulas, late windows, widths, counts and point series. The principal dense phases use accepted scalar types. The tests often reuse a selection. They assert results, not the cost of first entry. |
| Late/missing/gap tests, `tests/conformance.rs:2686,2707`; native fallback test, `src/backends/shm.rs:567` | Exercise rejection, missing initialization, timed errors and fallback handoff. The late fixture ends at tick 8 and has a few records. Correct all-batch fallback is expected and passes. |
| Replay tests, `src/query/engine.rs:2056` | Count native API entries to verify repeat-cache hits and invalidation. One entry can collect millions of records. They do not count records or full hierarchy imports inside that entry. |
| Certified/cached point tests, `src/query/engine.rs:2221,2460,2576` | Check chronological-reference parity, owned values, same-tick state, projections, callback cleanup and zero/one/two native API entries. Typed examples end at tick 7; wide examples have 4096 ticks. The imported reentrant clock sample is at tick zero. Correct fallback can satisfy entry-count assertions without a bound on collection work. |
| Generated streaming tests, `src/query/engine/streaming_tests.rs:171,260` | Bound shared-engine slots, generated-reader advances and early stop. `probe:29` recognizes only `Reader::Generated`. It excludes SST2 work and the C++ prefix vector. |
| SHM `histories` and matched clock, `benches/shm.rs:113,395`; `benches/support/matched_clock.rs:28` | Include genuine cold-reader/fresh-selection points and a million-transition clock. These use the accepted authored scalar; they do not reproduce the rejected dense scalar. This is not a suite containing only small files or warm queries. |
| SHM `window`, `benches/shm.rs:81` | Opens/resolves outside timing and primes the same selection with scan/candidates. Repeated complete scans/traces can resume. First-change stops can invalidate state and incur setup again; the cases still use controlled accepted types or short histories. |
| Banks, topology, wide and typed, `benches/shm.rs:232,288,353` | Cover 1/16/64 bases, 16,384 unrelated values, projection fallback, wide payload and rich values. Fresh quiet scans do expose hierarchy setup. They do not combine a long rejected dense prefix with large point batches or staged disjoint trigger/payload calls. |
| Point batches/aliases | Authored principal point batches have four bases; matched repeats use four duplicate slots. Pool alias checks validate identity when available, or separate values otherwise. They do not measure the 100/419-base rejected point combination. |
| Composed workload, `tests/support/shm_workloads.rs:89` | Measures selective/eager access inside one prepared selection. Native payload may already be collected. It is not two one-shot calls on disjoint selections over a large hierarchy. |
| Criterion acceptance, `docs/benchmarking.md` | Timings are manual. Saved baselines compare the same case against itself across revisions; an initially slow implementation can remain unchanged. No absolute limit or cross-format 2x target exists. FSDB/SHM matched cases share selected histories, but no recipe computes or gates their ratio. |

The available release log contains `Testing ... Success` smoke results, 78/78
SHM pool passes and no skips. No timed SHM Criterion baseline was found in the
examined main-checkout outputs; only smoke/build evidence was found. There is
no evidence that a statistical threshold, filter, noise classification or human
review suppressed a recorded SHM regression. The supported explanation is a
manual measurement policy plus workload boundaries and initial-baseline limits.
Compilation and successful smoke execution do not provide timing evidence.

A fresh manual `bench-shm matched-clock` capture here supplies a control: the
qualified cold-reader point estimate is 14.9 ms, prepared duplicate-slot repeat
92 ns, fresh-selection scan 17.9 ms and prepared scan 2.8 ms. This creates a new
local baseline, with no earlier baseline comparison. It confirms that these
existing workloads distinguish setup and reuse, while using the accepted type.

## History, rejected explanations and limits

SHM first entered main in `20c9c5c` and first shipped in 1.1.0. The investigated
native code, adapter and benchmark target are unchanged between that addition
and the release checkout. There is no older released Ondas SHM implementation
against which to date a regression. The costs are present in the initial
implementation; a regression in vendor storage or an earlier private prototype
has not been established.

The reproduced costs exclude wavepeek resolution and rendering. They reject
adapter indexing, per-signal API calls and equality-expression work as the
explanation for these point cases. The 1000-position case is one batch, and a
preparing point fallback does not perform a second native collection. Staged
calls have distinct requested histories after removing the covered trigger.
The Python wait-wrapper artifact explains some small CLI ratios, not millions
of native records or multi-second queries.

Field mismatch is proven. It must not be relabeled a converter bug or a type
unique to converted VCD: imported corpus files also have that representation.
The exact private SST2 meaning of every property flag, formatter availability
rules across all supported SDKs, SDK internal load/decompression costs, memory
high-water use and lock contention remain outside the measurements. No broad
SDK matrix or native-simulator/converted-file equivalence claim follows merely
from opening files. The proprietary format is not a public type API.

## Minimal follow-up and acceptance

1. Add cold point assertions for an existing oracle-listed rejected scalar and
   an eligible packed history, both alone and in one batch. Derive expected
   values and normalized `changed_at` from complete independent windows. Include
   order reversal, duplicates and a projection. Retain missing/unavailable,
   nine-state and same-tick net-equal cases as mandatory correctness controls.
2. Add one pinned dense converted control paired with a native authored control.
   Benchmark cold points for 1 and many unique bases, then add one rejected base
   to the eligible batch. State distinct histories and output positions in IDs.
   Keep the existing realistic PicoRV32 consumer as a local acceptance workload.
3. Add fresh-selection equal-width early/late windows for the rejected class,
   with scan, traces and Break-after-50 on identical bounds. Keep prepared
   repeat controls separate. Add one existing large-topology union versus two
   disjoint one-shot reads; include open and destruction explicitly.
4. Capture and review timed per-case baselines on each explicitly supported SDK.
   Preserve corpus/code/loader identity and an unchanged-revision control.
   Smoke must continue to check execution only. A cross-format 2x objective
   needs explicit adoption; it is not an existing Ondas release contract.
5. Before an algorithm change, agree private work counters for native collection
   and imports, including work before callbacks. Add deterministic regression
   assertions for the chosen safe strategy, rather than timing assertions in CI
   or generated-reader counters that cannot see the shim. A future point/window
   improvement must preserve error ordering, missing states, all nine logic
   states, final same-tick values, aliases/projections and `changed_at`.

Performance acceptance should use the same artifact, selection, bounds, setup
and destruction boundaries, with raw correctness checked outside timing. Require
improvement beyond measured unchanged-revision drift in each targeted cold case
and check the prepared controls for regressions. If adopting the external 2x CLI
objective, rerun paired direct CLI controls after the library change; native API
ratios alone cannot certify that objective. Keep the borderline DualRocket case
separate and do not interpret a current staged sequence as an algorithmic lower
bound. No optimization iteration is authorized by this note.

## Validation

`./dev just fixtures-install` verifies all 500 installed artifacts.
`./dev just ci`, `./dev just msrv` and `./dev just ci-shm` pass. SHM conformance
executes 78/78 pool fixtures with no failures or skips, 13 focused integration
tests and five native/replay library tests. The vendor gate also executes
benchmark smoke and standalone consumers on development Rust and MSRV.
The manual matched-clock capture succeeds. The 14 representative raw-value
and trace parity comparisons pass. The normal host commit hook supplies the
fixture-free `check-local` gate for this WIP-only contribution.
