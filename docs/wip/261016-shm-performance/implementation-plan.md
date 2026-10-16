# Reduce SHM cold-query work

This ExecPlan is a living document maintained under the `exec-plan` skill.

## Purpose / Big Picture


Cold SHM samples and late bounded reads currently copy millions of old records
before returning a few observations. The change must make the measured
PicoRV32, DualRocket, ClusteredRocket and SCR1 common-API requests materially
faster while returning exactly the same values, timestamps and errors. Clients
must not choose an SHM-specific query strategy.

## Non-Goals


Do not change the public API, retain vendor handles between calls, add a backend
framework, modify the published FST dependency, or push/publish/merge anything.
Vendor versions, credentials, generated inputs and machine settings remain in
ignored local files. Timing is a manual acceptance measurement, not a new CI gate.

## Progress


- [x] (2026-10-16) Verify branch `bb/shm-performance-thr_9ubsj62rqv` at `0842929` and the repository rules. Preserve the original investigation and inputs.
- [x] (2026-10-16) Rerun serial published-1.1.0 baseline into `tmp/shm-investigation/fix-baseline/` with all builds finished.
- [x] Prove safe point certification for the observed four-state scalar and integer representations against chronological reference reads, including gaps and same-tick states.
- [x] Implement supported point/window improvements, retaining ordinary fallback for unproven types and states.
- [x] Avoid full hierarchy reimport for selected queries using only owned declaration access recipes; keep SDK sessions private and short-lived.
- [x] Prevent one unqualified history from imposing its ordinary-prefix work on qualified histories in the same point request.
- [x] Add deterministic native-work regressions and focused cold benchmarks over pinned inputs.
- [x] Update backend, benchmark and public documentation where behavior descriptions change.
- [ ] Verify raw parity, SDK controls, repeated measurements, public CI, MSRV, vendor checks and doctests.
- [x] Run OCR Review and an independent Astra High review of the final diff; resolve substantive findings and rerun affected checks.
- [ ] Commit the verified result through normal host hooks and report measurements in Russian.

## Surprises & Discoveries


The original investigation found actual type rejection before expression work.
The representative converted scalar has kind 2, code 5, properties `0x7004060a`
and eight storage bits. The existing guard accepts code 3, properties `0x4060a`
and two-bit storage. A nine-state fixture uses a different code/property pair;
it must not be admitted by masking those differences. Packed integer declarations
also differ from the currently accepted packed logic type.

The same rejected scalar representation already occurs in pinned imported SHM
inputs. Their oracles mostly cover full windows from zero rather than cold
points. Existing native-read counters count API entries, not records collected
before a callback. New regressions must observe the latter work.

The first optimized SCR1 run remained slow because its clock has an initial
missing interval, rather than a value at tick zero. The equality expression
exposes that interval followed by a continuous available state. Certification
now accepts this shape and stops the backward change search at its first state.
It still rejects a later gap. The authored late-history fixture checks that its
first observation has no `changed_at` and earlier points remain missing.

## Decision Log


Decision: preserve `0842929` and the published consumer as the baseline; create
separate ignored consumers for current source and diagnostic prototypes.
Rationale: after/before comparisons need fixed source and measurement boundaries.
Date: 2026-10-16.

Decision: qualify only representations supported by evidence and retain the
availability certificate. Do not replace it with an unchecked seek.
Rationale: missing initialization, later unavailable intervals, unknown states
and normalized change times are observable contracts.
Date: 2026-10-16.

Decision: retain no SDK handles in query preparation. Investigate immutable,
owned enumeration routes captured when opening the public hierarchy.
Rationale: source files must remain unchanged; selected routes can avoid walking
unrelated hierarchy while preserving callback reentrancy and thread transfer.
Date: 2026-10-16.

Decision: return certified base samples separately from fallback rows and use
an immediate temporary selection over only the failed bases.
Rationale: the common Rust normalizer must retain ownership of same-tick final
values, missing states and change times. Copy certified bytes before the fallback
selection can release native buffers. Run no user callbacks until it succeeds.
Date: 2026-10-16.

Decision: count native enumeration visits and chronological records in regression
tests, while timing remains manual.
Rationale: callback-only counters cannot detect collection before a stop. The
new counters deliberately do not measure SST2 expression work.
Date: 2026-10-16.

## Outcomes & Retrospective


All 78 SHM fixtures, 14 focused integration tests and eight native query tests
pass on two selected SDKs after the review fixes.
Fourteen final raw cross-format comparisons pass, including entering change
times and staged/union traces. Fresh serial point/window medians improve by
11.8–16.1x in the original dense cases. Staged calls improve by about 2–3x in
query time. Unchanged-source and FSDB controls remain stable. The early window
and already-qualified phase controls show no material change. Astra High
completed both full and affected-fix reviews with no substantive findings. OCR
findings have failing-before-fix regressions. Completed OCR passes cover all
eight code files; no substantive findings remain. The clean-tree public release
dry-run is pending. Full measurements and residual limits are in
[implementation-results.md](implementation-results.md).

## Context and Orientation


`native/shm.cpp` owns short-lived SST2 sessions and copies native observations.
`point_type` restricts representations; `point_prefix` uses a source equality
expression to certify continuous availability; `certified_point` seeks the value
and walks backwards over equal states to establish its last normalized change.
`history` is the ordinary chronological path. Before this change,
`ondas_shm_sample_bits` required the complete batch to certify; on failure it
could collect ordinary rows for the following `ondas_shm_read` without opening
another session. The implementation now isolates this work per history.

`src/backends/shm.rs` copies native buffers under a process-wide lock and invokes
Rust visitors after releasing it. `src/query/engine.rs` normalizes final values
at each tick. A successful selection can keep one bounded state snapshot, not
a whole history. Positive-start windows first request an entering-state point;
failure currently leads to a second full hierarchy import and prefix collection.

`tests/conformance.rs` and `src/query/engine.rs` contain real-fixture checks for
missing states, errors, projections, same-tick changes, owned values, reentrancy,
thread transfer and panic cleanup. `tests/support/shm_workloads.rs` defines
benchmark consumers. `benches/shm.rs` and `benches/support/matched_*.rs` register
controlled workloads. The pinned fixture commit is
`4ce2f4ea6a034f1db335cc96ba9e67e6dcbb2a38`.

## Open Questions


The selected SDK controls and chronological references support the narrow
wire/integer qualification. Per-history fallback reuses the common normalizer
without synthetic records. Owned parent/property/ordinal/name routes cover the
pinned scope, array and alias cases. Current profiling locates most residual
dense-point time in SST2's certificate evaluation.

Broader SDK behavior, internal certificate work, peak memory and contention are
not characterized. A large independently-oracled delayed-initialization input
and a fresh full paired CLI matrix remain follow-up work.

## Plan of Work


### Milestone 1: establish point eligibility and cold-window entry


Use ignored native prototypes to inspect certification on the rejected scalar
and integer layouts. Compare points and entering states to explicit scans from
zero, including unknowns, delayed initialization, recording gaps, duplicate slots
and net-equal same-tick writes. Only promote a guard change after these checks.
An eligible positive-start query should then reuse the existing common-engine
entering snapshot and native bounded resume rather than collect its old prefix.
Document exact supported layouts privately in code and practical behavior in
`docs/shm-lib.md`.

### Milestone 2: reduce unrelated setup and isolate fallback work


Capture owned routes to native declarations during initial import in
`native/shm.cpp`. A route consists of enumeration properties and positions,
with names to validate traversal. Resolve only selected routes inside a fresh
session, sharing common parent work. Re-read and check the selected declaration
shape. Input immutability remains required. Metadata-only opening still skips
the hierarchy.

For mixed point eligibility, return copied certified states separately from
ordinary fallback inputs and let the existing Rust normalization path process
only those inputs. Any private protocol must release all buffers on error,
callback stop and panic. It must not turn a partial batch into successful output
before all fallible selected reads finish. Use a smaller proof-of-concept first
if the native/Rust handoff needs to change.

### Milestone 3: prevent recurrence and document measurement boundaries


Add real-fixture correctness checks for the rejected representation and mixed
eligibility. Observe actual native collection and selected hierarchy work with
private test accounting; count work before callbacks, not only delivered rows.
Reuse independent oracle windows or authored formulas for expected values.
Add focused cold point, late window, stop and staged/union benchmark cases over
pinned fixtures, with prepared-repeat controls labeled separately.

Update `docs/shm-lib.md`, `docs/benchmarking.md`, `docs/testing.md` and affected
Rustdoc to describe the final implementation accurately, using practical STE.
Keep investigation history in this WIP directory and raw outputs under `tmp/`.

### Milestone 4: validate and review


Run relevant SHM tests, then the complete public and vendor gates. Build the
current-source consumer before timing. Run raw comparisons outside timing and
the same serial baseline matrix on an idle machine. Inspect all per-case results,
including opening, query, resolution and destruction separately. Run OCR Review
with SHM semantics and performance context, then an independent read-only Astra
High review. Resolve substantive findings and repeat affected checks before
committing with normal hooks.

### Concrete Steps


Run project commands from the repository root through `./dev`. Existing local
profiles and hooks are installed; inspect their ignored setup rather than
putting SDK details in tracked text.

    ./dev just fixtures-install
    ./dev env CARGO_TARGET_DIR=tmp/shm-investigation/build tools/repo/shm-sweep python3 -B tmp/shm-investigation/run.py measurements fix-baseline
    ./dev just conformance-shm
    ./dev just bench-smoke-shm
    ./dev just ci
    ./dev just msrv
    ./dev just ci-shm
    ./dev cargo test --doc --locked

The first two commands have already supplied installed inputs and the new
baseline. Output directories are unique; choose a fresh suffix on repeated
captures. A successful SHM pool currently reports 78 passed and zero skipped.
Update this section with focused regression and consumer commands as they exist.

### Validation and Acceptance


Every representative sample and full trace must retain exact raw values and
timestamps; verify entering `changed_at`, not only counts. Existing missing,
unavailable, nine-state, same-tick, alias/projection, reentrant and panic tests
must pass. Native-work assertions must fail on the baseline path and pass on
the optimized path for the supported cold cases. Keep timing assertions out
of CI.

The targeted cold queries must improve beyond unchanged-revision drift, with
large-prefix cases no longer materializing millions of records when a valid
certificate permits direct entry. Large-hierarchy staged reads must avoid
reimporting the complete hierarchy on every selected operation. Review prepared
and early-window controls for regressions. Record residual SDK costs explicitly.
Historical CLI budgets remain context; do not present native API measurements
as a fresh paired CLI acceptance run or as an algorithmic lower bound.

### Idempotence and Recovery


Keep baseline binaries, source copies and outputs unchanged in ignored scratch.
Use separate Cargo targets for SDKs and prototypes. Repeat failed validations
after correcting the concrete failure. Do not reset user work or bypass hooks.
Rollback an unsuccessful prototype by discarding only its own ignored directory
or its reviewed source diff. Do not remove fixture payloads or shared evidence.

### Artifacts and Notes


`tmp/shm-investigation/fix-baseline/` records the fresh three-run baseline.
`tmp/shm-investigation/waveforms/` contains preserved SHM/FSDB database copies.
`tmp/shm-investigation/inputs/` names exact selections and bounds.
`docs/wip/261016-shm-performance/investigation.md` preserves the completed
investigation; append implementation results separately without rewriting that
historical source snapshot.

### Interfaces and Dependencies


Use the existing SST2 exports and private C++/Rust bridge. Keep all new route,
partial-point and accounting types private to the SHM backend. No third-party
dependency changes are planned. The Rust common engine remains the owner of
normalization semantics. The final private handoff must state its success,
partial/fallback and cleanup rules in `native/shm.h` and matching Rust comments.

Plan created 2026-10-16 after the user authorized implementation and requested
OCR plus Astra High at completion.
