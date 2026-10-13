# Optimize native GHW with measured, simple changes

This ExecPlan follows the exec-plan skill. It is a living document. Keep Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective current.

## Purpose / Big Picture

Improve opening and public waveform queries for GHW without changing returned values, error boundaries, or public types. Establish timings on the current implementation, test one optimization idea at a time, and keep changes only when repeated measurements show a useful improvement at an acceptable code cost. The user can verify the outcome through existing Criterion benchmark cases and the independent GHW conformance suite. The final report must show initial and final results, environmental variation, rejected ideas, and comparable FST or FSDB workloads where their timing boundaries permit a comparison.

## Progress

- [x] (2026-10-13) Started the optimization goal; confirmed clean initial revision a1f1ff6 and fixture revision 4ce2f4ea6a034f1db335cc96ba9e67e6dcbb2a38.
- [x] Requested read-only Astra high hypothesis reviews for decoding/opening and selected queries.
- [x] Initial private GHW tests passed (25 passed) and GHW conformance passed (204 file/bytes cases, 0 failed, 0 skipped).
- [x] Recorded the complete exploratory ghw-initial baseline and the unchanged focused ghw-initial-repeat baseline; preserved the initial executable under ignored tmp/ghw-performance/initial-ghw.
- [x] Hypothesis 1 minimal buffering patch passed 25 private tests, seven public GHW tests and all 204 GHW file/bytes conformance cases. Astra found no production defect; tiny-buffer tests now set the concrete wire buffer capacity.
- [x] Measured buffering and two old/new pairs in opposite orders. Accept the tiny production change for repeated scalar/dense decode and real-file opening gains; wide-query results remain noisy/neutral.
- [x] Buffering Astra and OCR reviews completed. OCR found only a test-helper precondition comment, which is now documented.
- [x] Committed buffering as 6f9768f after host quality hooks.
- [x] Hypothesis 2 passed 25 private tests, GHW conformance (204 cases), and Astra review. Two opposite-order pairs confirm short whole-vector scans 1.20–1.29x and repeat-point series 1.26–1.40x versus buffering alone; opening is unchanged.
- [x] Hypothesis 2 OCR completed with zero findings.
- [x] Committed hypothesis 2 as 83c1bcd after host quality hooks and zero OCR findings.
- [x] Added a public repeated-ID partial-CYC initialization test and confirmed it passes before the structural change.
- [x] Hypothesis 3 passed 25 private tests, eight public tests, 204 conformance cases, Astra and OCR (zero findings).
- [x] Two main pairs confirm early whole scans 1.89–2.06x and repeat series 1.99–2.25x versus hypothesis 2. A noisy many-scalar case was repeated in two additional pairs: scan/1 improved 1.45x/1.50x and scan/64 1.29x/1.25x. Accept flat dependencies.
- [x] Committed flat dependencies as b39e87d after quality hooks.
- [x] Hypothesis 4 full descriptor trial passed tests/Astra but paired results were mixed: some wide queries improved 10–22%, opening was neutral or ~5% slower, dense controls were inconsistent. Saved its patch/binary and removed the full descriptor change.
- [x] Minimal effective-type normalization passed 25 private, eight public and 204 conformance cases; Astra confirmed semantics. Opposite-order pairs show repeat series +12–16% file and +6–8% bytes, with other cases noisy or neutral. Accept this small map-only refinement with scoped claims.
- [x] Effective-type OCR completed with zero findings.
- [ ] Commit effective-type normalization. Then test a projected-value copy shortcut with an active-wide counterexample.
- [ ] Rank hypotheses against observed costs and check one minimal change at a time.
- [ ] Review, validate, and commit each accepted optimization; record rejected experiments.
- [ ] Re-measure final representative cases against the initial implementation and analogous FST/FSDB cases.
- [ ] Run final gates, write the report, promote durable backend conclusions, and remove this tracked WIP after saving an ignored copy.

## Surprises & Discoveries

Opening validates the full body. A warm query currently builds dependencies and selected buffers anew and replays from the first body section. Each byte read invokes the boxed input's buffer access and consumption. This is source inspection evidence, not a measured bottleneck yet.

Astra confirmed that a selected 4096-bit vector currently creates about 4096 small dependency vectors and performs a second hash lookup for first-seen state. A buffered concrete input, merged seen state, and flat dependency positions are the first three candidates. Resolving scalar decoding metadata is a later separate candidate.

Existing scan-first cases stop on the first callback, which can be an Initial record. They are setup/first-callback cases, not first-transition measurements. Prepared point series contain three visits per iteration; a forward series rewinds between iterations. Real-recording selected prefixes currently have little verified activity. Report them as real-file prefix controls, and use full opening for decoder evidence.

The start of the exploratory full baseline overlapped the tail of the initial conformance process. Do not use the first file opening/hierarchy cases as final baseline evidence. The unchanged focused repeat will re-measure every case chosen for the final report after validation is fully complete.

Unchanged process runs show material environmental variation. Examples of Criterion mean estimates: wide late scan 2.90 ms to 2.16 ms; scalar late scan 0.82 ms to 0.72 ms; real ws2815b opening 110 ms to 42 ms. Keep the focused repeat as the initial comparison for matching cases, and require adjacent old/new runs for conclusions. Raw exploratory numbers alone are not causal evidence.

## Decision Log

Use the existing branch and pinned corpus. Keep raw outputs, machine details, scripts, and Criterion results in ignored tmp/ or target/. Do not change waveform data. Rationale: changing the workload would invalidate the comparison and infrastructure details do not belong in Git.

Run only one timed benchmark process at once. Run builds, tests, reviews and agent source inspection outside timed measurement where practical. Compare unchanged runs and alternate an old and new build for final checks if drift is material. Rationale: the user has other concurrent tasks, so a statistical label from one run is insufficient evidence.

Use Astra high only for hypothesis search and focused review. Review agents may not edit or run expensive processes. The root implements and measures one hypothesis at a time. Rationale: source reviews can run independently without combining unmeasured changes or competing timed workloads.

Hypothesis 1: put a concrete standard BufReader around Wire's boxed input, leaving checked reads and absolute seeks unchanged. Expected benefit: amortize virtual input calls over chunks during opening and replay. Controls: bytes input and small/scalar points. Risks: extra copy/bounded buffer, seek invalidation, tiny inner buffers and injected I/O errors. Test before timing.

Accept hypothesis 1 after two adjacent pairs with opposite order. Real ws2815b opening improved by 1.31x and 1.45x. Dense constant replay improved by 1.37x and 1.36x; file scalar late scan by 1.28x and 1.21x. Wide-query ratios varied from modest wins to about 5% regression and do not support a separate claim. Production cost is one standard bounded input buffer and three changed lines. Existing record budget and I/O tests still pass. OCR precedes the commit.

Hypothesis 2 keeps the dependency map and position vectors, but puts the seen flag beside the positions. It removes the separate HashSet and its per-selected-record lookup. Expected gains are wide setup/short-prefix queries and repeated points; opening is an unchanged control. All dependent positions must share one first-seen result, including repeated IDs. This is independent of flattening positions or retaining state.

Accept hypothesis 2 from paired evidence, not its more optimistic exploratory sweep. Whole early scans improved 1.26x/1.20x in file mode and 1.26x/1.29x in bytes. Repeat-point series improved 1.27x/1.26x (file) and 1.40x/1.38x (bytes). Dense 64-probe scan improved about 1.10x/1.11x; full late whole scan has only a small or inconclusive gain. Opening controls moved within about 3%. Keep each claim scoped to the affected setup work.

Hypothesis 3 uses one sorted Vec of source-ID/slot/position entries, with the seen flag on the first entry of each equal-ID group. A lower-bound partition_point finds that first entry; do not use an arbitrary binary-search match. This removes per-scalar position-vector allocations and hash-table storage while keeping memory proportional to selected positions. It adds logarithmic lookup work for every decoded source record. Measure dense/many topology, long whole prefixes, scalar controls and early wide setup before acceptance. Preserve list/tick order and all selected positions.

The added public test changes independent literal HIE recipes to [3,3,4] and [4,3,3], omits SNP, writes ID 3 twice before ID 4, and checks Missing at ticks 0/1 and complete values at tick 2 across backward reads. It protects initialization and grouping behavior rather than internal data structure details.

Accept flat dependencies after repeated wide setup and miss-heavy controls. Main pair 2 had pronounced late-run noise, including a 7% apparent regression in many-scalar scan/1 and large shifts in unchanged bytes cases. Two extra opposite-order miss-focused pairs instead showed scan/1 gains of 1.45x/1.50x, scan/64 1.29x/1.25x, and dense constant gains 1.38x/1.45x. Opening controls are unchanged within the observed variation. Long whole scans are near neutral or modestly faster, not the main claim.

Hypothesis 4 replaces the existing active table with SourceScalar descriptors (source ID, Storage, base literal count as usize). Resolve types once at hierarchy completion; remove repeated Types::scalar/alias traversal from body decoding. This does not add another index or retain values. Keep original types and recipes for declaration metadata and selected assembly. Preserve the exact ordinal, signed width and real-bit checks; 256 enum literals must remain representable. Tests of the body state machine now construct explicit descriptors instead of parsing a synthetic type table; full corpus tests still parse original types/hierarchy.

Reject the full descriptor form for its larger source/type changes without a sufficiently broad stable improvement. Test the smaller refinement alone: map (source_id, type_id) to (source_id, types.effective(type_id)) when building the active table. Keep recipes' original type IDs and all decoder code unchanged. Types::scalar already applies effective before decoding, so this preserves the base ordinal count, WKT, numeric widths, metadata and offsets. Astra confirmed that the graph is immutable after WKT and hierarchy parsing. The minimal transformation costs one map operation, adds no table or cached state, and can recover alias-heavy wins with much lower code cost. It still requires paired evidence before acceptance.

Accept the effective-ID refinement for its low code cost and repeated point-series gains: file 708 to 613 us and 588 to 527 us per three visits; bytes 569 to 537 us and 614 to 569 us. Broad exploratory runs were heavily contaminated by load and are not evidence of regressions. Other paired cases have neutral or noisy results, so do not claim faster full opening or general dense throughput from this step. Preserve full descriptor rejection separately rather than combining the two implementations.

Do not require a universal numerical acceptance threshold. Prefer repeatable double-digit improvements or changes that remove substantial work with simpler code. Treat a few-percent change as noise unless controlled repeats distinguish it clearly. Check regressions in unaffected cases. Rationale: implementation cost and absolute workload cost matter alongside ratios.

## Outcomes & Retrospective

No optimization has been accepted yet.

## Context and Orientation

The library is a read-only Rust waveform API. GHW stores scalar snapshots and time cycles, while Ondas exposes whole signals and projections. src/backends/ghw/wire.rs reads checked fields from a boxed buffered and seekable input. src/backends/ghw/types.rs reads type definitions and constraints. src/backends/ghw/hierarchy.rs resolves declarations into ordered scalar recipes: a recipe lists the source scalar IDs needed for one whole signal. src/backends/ghw/body.rs validates and replays source changes and completed ticks. src/backends/ghw/mod.rs assembles selected whole values and feeds the common query engine. That engine must preserve final equal-tick state, projections, duplicates, events and callback termination. Reader-private state must remain private.

benches/ghw.rs measures file and shared-bytes inputs, opening, hierarchy, selection, wide vectors and slices, point schedules, scans, owned traces, composed queries, topology controls and real recordings. A prepared query excludes opening and selection creation. A fresh-selection query includes selection construction. Neither label currently implies retained source histories. tests/support/ghw_workloads.rs owns paths and bounds. tests/support/ghw_workload_checks.rs and tests/ghw_native.rs verify consumers and independent observations. Private module tests cover malformed fields, tiny buffers, aliases, endian variants and bounded assembly. The fixture suite covers 102 GHW inputs in both input modes: 98 positive sparse profiles and four malformed files.

benches/fst/hotpaths.rs and benches/fsdb/controlled.rs contain similar history, topology and width controls. Those formats can have different ticks, source activity, loading costs and retained-state semantics. Report these differences instead of claiming equivalent formats. The FSDB target requires the fsdb-lib feature and an existing local SDK profile. Do not install or change proprietary infrastructure just to make a comparison.

## Open Questions

The initial timings must distinguish decoder overhead from dependency allocation, whole-vector processing, and prefix replay. Determine whether a bounded checkpoint or input buffering is worthwhile only after measurement; do not promise either design in advance. FSDB comparisons depend on an already available SDK environment.

## Plan of Work

### Milestone 1: baseline and measurement controls

Validate the current GHW state, then measure the existing complete target with shortened exploratory Criterion settings. Save an initial named baseline. Repeat unchanged representative cases under a second name to estimate variation. Identify expensive user operations and controls, then choose focused cases for each experiment. Record exact commands, commit, corpus, measurement settings and intervals in ignored logs. This milestone is complete when opening, wide/scalar/projection queries, sparse/dense topology, callback stopping, and realistic recordings have measured initial values and the source-review hypotheses can be ranked against them.

### Milestone 2: one hypothesis per experiment

Write the proposed cost reduction, expected cases and semantic risks in the live log before editing. Make a minimal change in the existing backend. Run the smallest relevant unit/public tests and GHW conformance before measuring. Compare the affected cases with their initial and immediately previous baseline, and include cases expected to remain unchanged. Repeat meaningful wins; reject changes whose effect does not survive noise or whose state machinery costs too much complexity. Record source and measurement evidence. Ask an Astra high reviewer to check each accepted patch, especially malformed input, equal ticks, sparse scalar IDs, ordered overlapping recipes and callback stopping. Use OCR on accepted milestones with explicit backend/measurement context. Commit only validated changes through host hooks. Do not patch published FST dependencies or introduce a generic backend framework.

### Milestone 3: final comparison and report

Measure the final complete GHW target with the same exploratory settings. Re-run the most important cases with longer measurement and an alternating initial/final order when necessary. Keep both binaries outside timing to avoid rebuild load during measurement. Measure a small set of existing analogous FST cases and FSDB cases if the configured SDK is available. Explain activity and setup differences. Run the complete project gates once the final implementation is stable. Update docs/ghw.md only for durable storage or query-cost changes and review its plain English and accuracy. Write a self-contained final report under ignored tmp/ghw-performance/ and provide a clickable link and concise result table to the user. Save this plan there and remove only this task's tracked WIP directory before completion.

## Concrete Steps

Run project commands from the repository root through ./dev; keep Git on the host. The worktree already has installed hooks and a running container. The initial pin is a1f1ff6 (full SHA to be captured locally), and fixture SHA is 4ce2f4ea6a034f1db335cc96ba9e67e6dcbb2a38.

    ./dev cargo test --locked ghw
    ./dev cargo test --locked --profile conformance --test conformance ghw -- --ignored --nocapture
    ./dev cargo bench --locked --bench ghw -- --save-baseline ghw-initial --sample-size 20 --warm-up-time 0.2 --measurement-time 0.5 --nresamples 10000

For a focused unchanged repeat, pass a Criterion case-name filter and save a new baseline. Keep IDs and settings identical for old/new comparisons. For example, the whole-vector early scan is ghw/ghw0097-wide-toggle/file/scan/whole/early. Use its exact ID as a filter and Criterion's --exact flag when appropriate. The output must include a time estimate and interval; no skipped input can count as measured evidence.

    ./dev cargo bench --locked --bench ghw -- 'ghw/ghw0097-wide-toggle/file/scan/whole/early' --exact --baseline ghw-initial --sample-size 30 --warm-up-time 0.5 --measurement-time 1 --nresamples 10000

After each patch, use its relevant tests and the GHW fixture checks before timing. At completion:

    ./dev just ci
    ./dev just msrv
    ./dev cargo test --doc --locked

OCR runs on the host against the intended commit or diff, with --audience agent, --background or --background-file, and --output under tmp/ghw-performance/. Do not expose credentials in output. Run reviews outside timed measurement.

## Validation and Acceptance

All existing GHW public, private and fixture checks must pass. Fixture conformance must still report 204 passed, 0 failed, 0 skipped. Preserve both input modes, source ordinal validation, integer overflow errors, tiny-buffer reads, complete opening validation and early callback boundaries. New tests must cover actual behavioral risks of an accepted change, rather than mirror its implementation.

An optimization is accepted only when the same benchmark IDs and artifact hashes show a repeatable improvement, affected workloads have a plausible causal explanation, relevant controls do not show unexplained material regressions, and review finds the code cost reasonable. The final report must disclose noisy or inconclusive cases and the input/retained-state limits of cross-format comparisons. Completion requires a clean worktree and local commits; no push, release or fixture changes are requested.

## Idempotence and Recovery

Criterion result names distinguish each run; do not overwrite the only initial evidence. Before discarding a hypothesis, save its small patch and results under ignored tmp/ghw-performance/. Revert only this task's own edits. Keep other users' files and running tasks intact. Do not change process affinity, system services or global settings to conceal contention. A failed test blocks measurement claims until fixed or the hypothesis is removed.

## Artifacts and Notes

Ignored tmp/ghw-performance/ holds logs, compact summaries, experiment patches and the final report. target/criterion/ holds Criterion estimates and named baselines. A baseline is the recorded time distribution for a fixed case on a fixed code revision, not a performance guarantee.

## Interfaces and Dependencies

Keep the public Ondas API and production dependencies unchanged unless concrete evidence demands a separately reviewed choice. Existing private GHW types may change internally. Use Criterion's existing target and result format instead of adding a benchmark manifest, dashboard, generic profiler or custom history store. Any retained state must have an explicit bounded ownership and reset policy and must preserve current observable results.

Revision note (2026-10-13): created the measured optimization plan and validation boundaries before experiments.

Revision note (2026-10-13): recorded initial validation, ranked source-review hypotheses, benchmark interpretation limits, and the need to replace overlapped exploratory cases with an unchanged focused baseline.

Revision note (2026-10-13): saved the two initial timing sets and executable, documented drift, and recorded buffering validation before its timed comparison.

Revision note (2026-10-13): accepted buffering from repeated decoder-focused evidence, scoped its claims, and started milestone OCR before committing.

Revision note (2026-10-13): buffering review completed without production findings; documented the test-only buffer reset precondition from OCR.

Revision note (2026-10-13): recorded buffering commit and began the separate seen-state simplification experiment.

Revision note (2026-10-13): accepted combined seen/dependency state from two paired runs and recorded conservative effects and unchanged opening controls.

Revision note (2026-10-13): committed seen-state simplification, verified the new partial-initialization behavior test on that version, and began the independent flat-position experiment.

Revision note (2026-10-13): accepted flat positions after additional noisy-control checks and clean peer/OCR review, preserving all test evidence and the intermediate binary.

Revision note (2026-10-13): recorded flat dependency commit and began one-time source decoding descriptor resolution after Astra confirmed the active table previously retained original alias IDs.

Revision note (2026-10-13): saved and rejected the full descriptor experiment, and replaced it with the independently measured minimal effective-type-ID normalization trial.

Revision note (2026-10-13): accepted the minimal normalization from repeated query evidence, with no new descriptors/state and explicitly limited performance claims.
