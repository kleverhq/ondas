# Make cold VCD opening and first queries competitive

This ExecPlan is a living document. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` current. It follows the local `exec-plan` skill. Paths are relative to `ondas-lib`; temporary logs and comparison worktrees belong under ignored `tmp/`. This tracked plan belongs under `docs/wip/` only until its conclusions are promoted and the branch is merged.

## Purpose / Big Picture

Ondas users should be able to open large RTL VCD files and make a first late query without waiting several times longer than the existing Wellen-based workflow. GitHub issues [#31](https://github.com/kleverhq/ondas/issues/31) and [#32](https://github.com/kleverhq/ondas/issues/32) report separate costs: eager full-body validation and metadata extraction during `open()`, followed by a second full-prefix traversal for a cold late `samples()` or narrow `traces()` call. Establish repository-owned public workloads, measure old and new behavior, improve both costs in that order, and preserve immediately available time bounds, hierarchy, invalid-input rejection, values, events, and query timing.

## Non-Goals

Do not defer VCD body validation or `metadata().time_span()` to the first query, silently use Wellen as Ondas's reader, import private waveform evidence, add performance thresholds to CI, or claim identical Wellen/Ondas validation contracts. Do not push the library repository without permission. No subagents may implement either issue; read-only Astra Medium reviewers are permitted.

## Progress

- [x] (2026-09-26) Publish `kleverhq.ondas-fixtures` v4.8.0 (`919f671`) with `vcd0098-picorv32-test-vcd`, converted from public `fst0013`, alongside existing `vcd0097-scr1-max-ahb-coremark`. The new VCD has an empty oracle, so its benchmark validates the artifact and successful opens but is not a positive oracle-conformance case.
- [x] (2026-09-26) Add direct Wellen/default and Wellen/single-thread open controls to the SCR1 benchmark, add PicoRV32 controls, and measure before modifying the parser. The pre-change AHB Criterion open estimates were Ondas 3.071 s, Wellen 0.781 s, Wellen single-thread 3.230 s. The PicoRV32 historical estimate was Ondas 1.670 s, Wellen 0.535 s, Wellen single-thread 2.021 s.
- [x] (2026-09-26) Prove a first, insufficient serial improvement: combined whitespace/token reading and a bounded direct identifier-code lookup reduced AHB to 1.964 s (about 36%) and PicoRV32 to 1.168 s (about 30%). Synthetic dense/sparse identifier tests, existing VCD tests, public FST/VCD conformance and benchmark smoke passed. An initial `just ci` reached only the final clean-tree-only package check; `cargo publish --dry-run --locked --allow-dirty` passed. Do not call this a near-Wellen fix.
- [x] (2026-09-26) Prototype safe parallel file-body validation using independent positional reads from the original file handle, with a serial fallback and byte-input compatibility. AHB measured 0.472 s versus default Wellen 0.817 s; PicoRV32 measured 0.279 s versus 0.569 s. Focused chunk-join tests and the 99-fixture VCD pool (196 positive file/bytes cases; the new empty-oracle VCD skipped) passed. These are encouraging but not yet a reviewed or committed fix.
- [x] (2026-09-26) Address both preliminary Astra Medium findings: fallible worker creation now returns to the serial pass instead of panicking under thread exhaustion, and the fixture-free boundary test creates its ignored scratch directory. Focused tests, Clippy and both same-file open controls passed after those changes.
- [ ] (2026-09-26) Commit the #31 fix, pass the clean-tree `just ci` gate, then obtain the final read-only Astra Medium review and address any new substantive findings.
- [ ] For #32, first add and time a fixture-backed **cold first query** benchmark after `open()` on PicoRV32: roughly 73 distinct public physical handles from the issue's 100 selectors at ticks 0, 1,225,000,000 and 2,455,330,000, plus a narrow late trace if useful. Keep `open` outside the timed query while measuring an independent end-to-end open-plus-query control.
- [ ] Improve the cold late point/range path without trading away the #31 opening gain or storing an unbounded signal-history cache; verify entering values, final-tick normalization, event counts, invalidation and repeated/early/backward queries. Re-run both #31 and #32 benchmarks on matched bytes and reverse the revision order when measuring noisy differences.
- [ ] Run `./dev just fixtures-install`, `./dev just ci`, `./dev just msrv`, and `./dev cargo test --doc --locked` after final commits; request an Astra Medium review of the #32 commit. Report remaining tradeoffs explicitly and leave the library unpushed.

## Surprises & Discoveries

- `wellen::simple::read` 0.25.6 *does* consume the VCD body and builds a time table before returning. Its default file reader maps the file and parses chunks in parallel; with `multi_thread: false`, AHB took about 3.2 s, similar to the old Ondas 3.1 s. A lazy Ondas pass is neither necessary to explain #31 nor contract-preserving.
- AHB's VCD header took about 2.6 ms; the body took roughly 3.13 s before the serial fix. An ignored, temporary profiling variant reduced body time to roughly 1.65 s when value lookup and checks were omitted, proving that skipping correctness is a tempting but unacceptable benchmark trick.
- On the user's concurrently busy machine, Wellen's default PicoRV32 timings varied between roughly 0.5 and 0.8 s. Compare old/new Ondas and stable Wellen controls in alternating order; Criterion confidence intervals and several independent processes matter more than a single ratio.
- A first attempt to scan whitespace with two `memchr` passes regressed AHB to about 3.1 s and was reverted. The serial improvement alone left Ondas noticeably slower than parallel Wellen.
- `src/lib.rs` denies unsafe code, so a memory-map prototype was rejected before running. Standard-library Unix `FileExt::read_at` lets multiple bounded buffered readers validate disjoint pieces of the same underlying file handle without a full-file allocation or unsafe code. Any failed or ambiguous piece is discarded and the original sequential reader returns the authoritative result. Fallible worker spawning also takes this fallback. Wellen controls drifted 2–6% during the parallel measurement, much smaller than Ondas's 83–85% improvement.

## Decision Log

- Decision: Retain the eager, fully validated `Reader::open` contract and seek throughput through a parallel file-only opening pass with a serial correctness fallback; keep the byte-input path serial unless a measured need justifies more work. Rationale: public `Metadata::time_span()` returns exact bounds immediately and opening rejects invalid bodies. Date/Author: 2026-09-26, main session.
- Decision: Use a practical near-parity target of about 1.5× default Wellen's same-file `simple::read` on **both** public dumps, not a universal hard-coded CI threshold. Rationale: the user explicitly rejected the remaining roughly 2–3× gap, and concurrent local workloads can shift wall-clock numbers. Date/Author: 2026-09-26, main session.
- Decision: Use safe standard-library positional reads, not a memory map, for the parallel file-only prototype. Rationale: the crate denies unsafe code, the same file handle can be shared safely through `FileExt`, and bounded per-thread buffers retain streaming memory behavior. Date/Author: 2026-09-26, main session.
- Decision: Address #32 only after #31 is benchmarked, committed and reviewed; preserve and recheck the #31 open time when changing first-query behavior. Rationale: moving the cold-query cost into opening would appear to fix #32 while making #31 worse. Date/Author: 2026-09-26, main session.

## Outcomes & Retrospective

Not complete. Both large VCDs currently meet the near-Wellen opening target in a prototype that still needs final review, gates and a commit; #32 remains untouched. The provider's new VCD has no independently generated VCD oracle; existing conformance and focused tests protect the shared parser, not a claimed new oracle for that asset.

## Context and Orientation

`src/waveform.rs::open_input` constructs file or byte inputs and selects the backend; `read_metadata` also invokes the VCD reader. `src/backends/vcd.rs::Reader::open` parses declarations and walks every value record to validate it, collect body comments, detect real declarations carrying strings and derive first/last ticks. It returns a reader that retains declarations, not a full history. `Reader::read_from` later seeks back to the saved body offset to answer a first query; `src/query/engine.rs` maintains prepared selection state and limited checkpoints. `benches/vcd.rs` loads and hashes locked artifacts through `tests/support/fixtures.rs`; `fixtures.lock.toml` pins provider v4.8.0. `tests/vcd_native.rs` covers hostile lexing and replay; `tests/conformance.rs` checks all nonempty public FST/VCD oracles and deliberately skips the new empty-oracle VCD.

Wellen controls in `benches/vcd.rs` use exactly `wellen = "=0.25.6"` as a development dependency. Default Wellen is multithreaded; its `LoadOptions { multi_thread: false, ..Default::default() }` control helps distinguish throughput from skipped work. Benchmarks create and drop a fresh waveform each iteration, normally with a warm operating-system page cache. A fresh release process is a separate sanity check, not a cold-disk guarantee.

## Open Questions

The prototype splits at newline-prefixed timestamp markers, validates every chunk through the same parser, joins timestamps, comments and real/string observations, and falls back to the original serial reader on an invalid or ambiguous join. An adversarial review must still check that no other legal or malformed cross-chunk state can silently evade that fallback. Once #31 is reviewed and committed, measure how much of #32's cold query remains and whether a bounded position/state index, reusing opening work or a different strategy can fix it without increasing open cost.

## Plan of Work

First preserve the present before/after Criterion case names and old checkout in ignored `tmp/issue31-source-before` (`2c069d9`), including the exact fixture SHA-256 values in output logs. The public tag is `v4.8.0`; no one should rewrite its contents. On AHB, add a file-only parallel opening prototype behind the existing VCD backend, while retaining the current serial `walk` for fallback and subsequent queries. Independent buffered positional reads must refer to the **same open file handle**, not a newly opened pathname, so path replacement cannot validate one file and query another. Partition at actual token boundaries, not arbitrary bytes, and join validated segments in source order: bound first/last times, enforce global monotonic timestamps, concatenate comments, reconcile string/real observations and ensure no dump block or comment crosses a cut. If any chunk or join is ambiguous or fails, re-run the original serial pass and return its actual result and error. Size direct-lookup tables relative to declared identifiers; do not cache full histories or retain worker buffers after opening. Verify that `open_bytes` and the existing query reader remain unchanged in behavior. Prototype and measure before promoting the implementation; keep only code that materially reduces both public RTL opening costs.

After the #31 change is stable, obtain the exact public selector paths described in issue #32 from Wavepeek v3.0.1's public benchmark catalog once and embed stable workload parameters in `benches/vcd.rs`; benchmarks must not fetch a consumer repository at runtime. A first-use benchmark must construct a new selection and make a late point query with `open` outside the timed region. Include a small-tick control and a second late time to expose prefix scaling; measure an end-to-end open-plus-query case too. Only after the before measurement should `src/backends/vcd.rs` and, if necessary, the shared query engine be changed at the common source of the replay. Retain read-only backend boundaries and bounded retained state. Prove a materially lower first-query cost without shifting it into opening, then finish documentation and review.

### Concrete Steps

From the `ondas-lib` root, use `./dev` for project commands. The current baseline files are ignored under `tmp/issue31-open/criterion-{scr1,pico}-before.log`, the preliminary current measurements are `criterion-{scr1,pico}-after-first.log`, and interleaved process logs are in that directory. Run `./dev cargo bench --locked --bench vcd -- 'vcd0097-scr1-max-ahb-coremark/file/open' --warm-up-time 1 --measurement-time 12 --sample-size 10` and the same command with `vcd0098-picorv32-test-vcd/file/open` for #31. Full tests: `./dev cargo test --locked --test vcd_native`, `./dev just conformance`, `./dev just bench-smoke`, then `./dev just ci` after committing (the package check rejects dirty trees). For #32, run the new focused case at early and late ticks before editing query logic, save an explicit old baseline, rerun in reverse order and report both the full open and first-query times. Use `./dev just msrv` and `./dev cargo test --doc --locked` after documentation changes. Host Git commits run with the current container started and the reviewed hook installed.

### Validation and Acceptance

For #31, both published VCDs must load with the same exact `Metadata::time_span`, hierarchy and full-validation error behavior as before. Unit and public VCD/FST conformance must pass, including malformed fixtures and small-buffer token tests. Matched release-process and Criterion comparisons must show Ondas opening both files within approximately 1.5× default Wellen on the same inputs, with Wellen single-threaded as an additional control; if the machine is noisy, repeat or pin measurements and disclose the spread. Do not equate a speedup against an unchanged serial Ondas baseline with satisfying the new near-parity requirement.

For #32, the committed benchmark must reproduce the original first-query prefix cost on the #31-complete commit: early queries should be cheap while later queries scale with the traversed history before the fix. After the fix, late first queries and an open-plus-query control must measurably improve on the same immutable fixture, with tests preserving output order and values, final-tick state, events and validity. Recheck #31 because a query optimization that front-loads work is a regression. At the end both fix commits and their Astra Medium read-only review outcomes must be reported; no library remote push occurs.

### Idempotence and Recovery

Do not mutate published fixture bytes, provider tags or sibling oracle repositories. A failed or ambiguous parallel attempt can be discarded without changing public APIs: return to serial `Reader::walk` and report that near-parity is blocked rather than merging a data-loss or invalid-input acceptance regression. Ignore `tmp/` artifacts and never commit downloaded waveforms. Temporary worktrees may be removed only after their logs and hashes are captured; do not remove another worker's files. Re-run `./dev just fixtures-install` against pinned v4.8.0 if a new checkout lacks data. Git hooks run on the host after `./dev` starts the current container.

### Artifacts and Notes

Observed before/serial-after Criterion estimates, in seconds:

    SCR1 AHB: Ondas 3.071 -> 1.964; Wellen default 0.781 -> 0.760; Wellen single 3.230 -> 3.259.
    PicoRV32: Ondas 1.670 -> 1.168; Wellen default 0.535 -> 0.537; Wellen single 2.021 -> 2.017.
    Preliminary parallel AHB: Ondas 0.472; Wellen default 0.817; Wellen single 3.298.
    Preliminary parallel PicoRV32: Ondas 0.279; Wellen default 0.569; Wellen single 2.092.

The actual published waveform IDs and SHA-256 values are `vcd0097-scr1-max-ahb-coremark` (`6ffd3f85a383e0f3910dfbb6977b4abb6d8ca26b5ad2a969a160f8f70546522f`) and `vcd0098-picorv32-test-vcd` (`e8f39947f6406c2f10d6ff0b1826af7ce11d67828ebfd431e165348d6be7246f`). No private artifacts are used.

### Interfaces and Dependencies

Keep the public `ondas::open`, `open_with`, `read_metadata`, `Waveform::metadata` and `Selection` contracts intact. Backend-only code uses standard-library scoped threads and Unix `FileExt::read_at` on the same file handle; it must not add unsafe code, a full-file allocation, a new backend or a cache API. `wellen` stays a benchmark-only pinned development dependency. If using a file-only fast path, `open_bytes` retains the serial validator, and successful results from both paths must agree across the public corpus and adversarial boundary tests. For #32 use existing query and checkpoint types before adding any state or dependency.

Change note: Added issue #32 as the next milestone and raised issue #31 acceptance from a partial serial speedup to near-Wellen opening, at the user's request on 2026-09-26. This explicitly prevents closing either issue based on a displaced or merely smaller cost.

Change note (2026-09-26): Replaced the memory-map hypothesis with measured, safe, bounded positional reads after the crate-level unsafe-code prohibition was confirmed. Updated progress, evidence, decisions and acceptance status consistently; review and a clean-tree gate are still required.

Change note (2026-09-26): Replaced the unresolved chunk-join question with its testable prototype and noted the remaining adversarial-review gate; added the measured parallel controls without claiming final success.

Change note (2026-09-26): Recorded the two preliminary Astra Medium findings and their fixes, while leaving the clean-tree CI and final commit review as open gates.
