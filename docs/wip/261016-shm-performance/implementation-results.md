# SHM cold-query results

## Scope and measurement boundary

The baseline is published Ondas 1.1.0, source commit
`1cf6a0e61f645ad70f507bab8c20fa03c78876b0`. The implementation follows the
investigation preserved in [investigation.md](investigation.md). The branch is
`bb/shm-performance-thr_9ubsj62rqv`; the fixture pin remains
`4ce2f4ea6a034f1db335cc96ba9e67e6dcbb2a38`.
The performance implementation is commit `60cede0`.

A standalone release consumer uses the common API with `shm-lib,fsdb-lib` and
the original locked dependencies. It uses no wavepeek engine. Three serial runs
per case follow all builds. Query time excludes open, simplified lookup and
drop. Filesystem caches can be warm. These measurements are not CLI timings or
an algorithmic lower bound. SDK identities, loader checks, exact inputs, raw
outputs and timing records remain in ignored local evidence.
The timed matrix precedes the review's empty-range cleanup; none of its inputs
uses an empty range. The rebuilt reviewed consumer passes the raw comparisons
again. The timing paths and performance algorithm are unchanged by that cleanup.

## Confirmed changes and measurements

The exact observed four-state scalar wire and integer layouts can use the
existing availability certificate. Nine-state and other unproven layouts retain
ordinary reading. A leading missing interval can precede continuous valid
availability; the first observation still has no known change time. A later gap
still rejects certification and produces the ordinary error.

Point qualification now works per base. Only unqualified bases collect ordinary
rows, in the same native session. Their temporary selection uses the existing
Rust normalizer; certified values are copied before that selection runs.
Sample callbacks receive no partial success if any selected read fails.

Opening captures owned enumeration routes. Selected operations resolve their
shared ancestors and validate selected declaration shapes. They no longer
import every unrelated subtree. Sessions remain short-lived and serialized;
no SDK handle survives a native operation.

| Common API operation | Baseline query, s | Current query, s | Speedup |
|---|---:|---:|---:|
| PicoRV32 samples, 1 position | 0.099089 | 0.006229 | 15.9x |
| PicoRV32 samples, 100 positions | 1.891100 | 0.121408 | 15.6x |
| PicoRV32 samples, 1000 positions / 419 unique IDs | 6.629455 | 0.459517 | 14.4x |
| DualRocket samples, 100 positions | 3.390875 | 0.287277 | 11.8x |
| PicoRV32 traces, 95 positions, late 8-us window | 1.874640 | 0.129463 | 14.5x |
| Same window, scan | 1.905405 | 0.132294 | 14.4x |
| Same window, query with Break after 50 callbacks | 1.885965 | 0.130596 | 14.4x |
| PicoRV32 early 8-us control | 0.006115 | 0.006309 | 0.97x |
| SCR1 single-clock windows, six cases | 0.040440–0.041569 | 0.002545–0.002824 | 14.6–16.1x |

Fresh FSDB query controls remain near the baseline: PicoRV32 1000 positions
0.110103 to 0.110621 s; late traces 0.045323 to 0.044995 s. Repeating the
unchanged published binary after the main measurements gives 6.591003 s for
PicoRV32 1000 positions and 0.040925 s for the SCR1 2-ns window. The large gains
are not explained by unchanged-revision drift or the earlier Python polling
artifact. The small early-window difference does not establish a regression.

The staged workflow retains the trigger trace, then reads disjoint remaining
signals from the same Waveform. This table includes open, query and drop, but
excludes lookup:

| Selection, late 32-us window | Baseline union / staged, s | Current union / staged, s |
|---|---:|---:|
| ClusteredRocket 100 positions | 1.005970 / 1.120615 | 0.428957 / 0.469432 |
| DualRocket 100 positions | 0.614991 / 0.672685 | 0.335574 / 0.361346 |
| DualRocket 10 positions plus trigger | 0.520927 / 0.570877 | 0.194890 / 0.220485 |

The original DualRocket small staged case remains a borderline historical
failure, not a robust native-only baseline failure. No new full wavepeek CLI
matrix was run.

The new Criterion cases also ran on both revisions with identical benchmark
source, opening outside measurement, 10 samples, 0.2-s warmup and 1-s requested
measurement time. The four qualified million-transition phases stay near
27–28 ms. Adding one unqualified two-state integer changes the cold batch from
615 to 247 ms. On the 16,384-value topology, union reads change from 32.1 ms to
345 us; two disjoint reads change from 64.6 ms to 656 us. The imported wire point
changes from 316 to 209 us. These are manual Criterion estimates, not CI limits.
An imported 100-ID batch at tick 90,000 changes from 658 to 585 us. It supplies
a larger-ID-count control with a short source prefix; it does not reproduce the
millions of records in the external PicoRV32 case.

## Coverage and acceptance

Fourteen raw comparisons pass again on the rebuilt post-review consumer,
including all selected point bits, every trace
entering value and change time, all 16,318 PicoRV32 changes, all six SCR1 traces,
and staged/union/FSDB traces with 96,602, 64,801 and 64,101 changes. Hexadecimal
rendering is not used as a lossless oracle.

The previous gap and its source references remain in the original investigation.
The added coverage addresses these specific boundaries:

- Nonempty SHM oracle windows supply fresh points at start, midpoint and end,
  including imported types whose sidecars list no point samples. Empty windows
  retain their original no-output checks and do not synthesize point times.
- Real wire and integer cases compare bits and normalized change times against
  chronological reads from zero. Private counters require no ordinary history
  collection for those cold points and bound collection for their late windows.
- A mixed five-position selection requires collection of only its one
  unqualified base, including reversed positions, duplicates and a scalar slice.
  A late-initialized base checks Missing and the first observation's change time.
- The topology case bounds enumeration work before callbacks, excluding most
  unrelated declarations. Existing API-entry counts alone could not do this.
- New benchmarks create a fresh selection per cold iteration and separate warm
  repeat, mixed eligibility, 100 unique bit IDs and staged disjoint calls. Smoke
  execution still has no timing threshold; timing remains an explicit manual
  comparison.

Both selected SDKs pass all 78 SHM fixtures with zero skips, 14 focused
integration tests and eight native query tests after the review fixes. The
primary SDK also passes `ci-shm`, including MSRV, benchmark smoke and standalone
consumers. Public tests, external conformance and benchmark smoke pass. Default
MSRV, doctests and the SHM/FSDB coexistence test pass.
Implementation commit `60cede0` passes the normal host hook and the complete
clean-tree `./dev just ci`, including external conformance, benchmark smoke,
package verification and the publication dry-run. No upload occurs. The repeated
default MSRV and doctests also pass. All validation uses `./dev`.

## Review

OCR identified two medium findings. An empty oracle window could underflow the
new midpoint calculation. An empty native read could leave prepared owned rows
armed after clearing the Rust handoff tag. Each issue has a failing-before-fix
runtime regression; both now pass on both SDKs. The empty read releases its rows
without opening an SDK session. The private header also states the zero-filled
fallback descriptor shape explicitly.

The two other initial low findings do not identify reachable failures. The
fallback result length follows the public one-sample-per-base contract, and
selected-route construction guarantees a resolved entry before lookup. No
speculative runtime checks or route refactor were added for those suggestions.

The initial OCR pass and a focused follow-up completed the core implementation
and fixed-file reviews. Both reported partial coverage because provider context
limits interrupted the benchmark group. A separate bounded-context pass
completed all three selected files with no findings. Together the completed
passes cover all eight changed code files, including the affected fixes and
header clarification. No critical, high or medium findings remain. Independent
Astra High reviewed the full change and then the affected fixes, with no
substantive findings in either pass. Raw review reports remain in ignored
evidence.

## Remaining limits and next work

The existing certificate and backward search still have SDK work. Native-work
counters bound Ondas enumeration and chronological copying; they do not measure
expression evaluation inside SST2. PicoRV32 419-ID query cost is still about
4.15 times the equivalent FSDB query, and the DualRocket point is about 4.66
times. This change does not establish that every historical 2x CLI budget is met.

A separate instrumented current-source copy locates this residual cost: a
PicoRV32 419-ID native point takes 0.4521 s, including 0.4357 s in
`point_prefix`, 0.00042 s in route resolution and 0.00084 s in 803 point seeks.
It collects no ordinary histories. For SCR1, certification takes 0.00218 s and
bounded native history collection 0.000086 s. These are diagnostic single runs,
not the uninstrumented medians above. The earlier rejection of expression cost
as the original cause still holds: the old type guard never called this
expression on those selections. After admitting the types, expression work is
the main remaining cost.

Non-scalar projections, unproven types, unavailable histories and other
incompatible selections retain ordinary reading. That path can still collect a
long prefix. Break still stops delivery after native collection; a qualified
entering state bounds that collection to the requested window, not 50 rows.

Further work should first profile the remaining certified SDK work and compare
safe alternatives with chronological references. Keep availability, same-tick
final values and projected change times as explicit acceptance criteria. Add a
large delayed-initialization fixture when the fixture project can supply an
independent oracle. Maintain timed SHM baselines and review cold/mixed/staged
results explicitly; smoke success cannot replace that review. Do not infer a
regression date from the absence of an older timed baseline.

## Local evidence

`tmp/shm-investigation/fix-baseline/` and `fix-final-measurements/` contain raw
three-run measurements. `fix-final-verify/checks.json` and
`fix-reviewed-verify/checks.json` index the original and post-review raw parity
outputs. `tmp/shm-fix/` contains unchanged controls, Criterion logs, validation
logs, diagnostic prototypes and review outputs. Existing sources and waveform
inputs remain available in this worktree; no SDK, target tree or waveform data
is tracked.
